use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Descriptor {
    UnixSocket {
        path: Option<PathBuf>,
        listens: Option<bool>,
    },
    /// A socket of any other kind, with what the system calls it. Whatever the listing cannot place
    /// counts as one, so that a kind nobody thought of fails a test.
    OtherSocket(String),
    /// macOS's system domain: a line from the process to its own kernel, which nothing else can
    /// connect to.
    KernelSocket(String),
    Pipe,
    Terminal,
    File,
}

pub fn descriptors_of(pid: u32) -> Vec<Descriptor> {
    if cfg!(target_os = "linux") {
        let unix = std::fs::read_to_string("/proc/net/unix").unwrap_or_default();
        let Ok(entries) = std::fs::read_dir(format!("/proc/{pid}/fd")) else {
            return Vec::new();
        };
        entries
            .filter_map(|entry| std::fs::read_link(entry.ok()?.path()).ok())
            .map(|target| from_proc(&target.to_string_lossy(), &unix))
            .collect()
    } else {
        let listed = Command::new("lsof")
            .args(["-a", "-p", &pid.to_string(), "-F", "ftn"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .expect("start lsof");
        from_lsof(&String::from_utf8_lossy(&listed.stdout))
    }
}

pub fn descendants_of(pid: u32) -> Vec<u32> {
    let running: Vec<(u32, u32)> = if cfg!(target_os = "linux") {
        std::fs::read_dir("/proc")
            .expect("read /proc")
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<u32>().ok())
            .filter_map(|pid| {
                let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
                parent_in_stat(&stat).map(|parent| (pid, parent))
            })
            .collect()
    } else {
        let listed = Command::new("ps")
            .args(["-axo", "pid=,ppid=,stat="])
            .stdin(Stdio::null())
            .output()
            .expect("start ps");
        assert!(listed.status.success(), "ps failed");
        String::from_utf8_lossy(&listed.stdout)
            .lines()
            .filter_map(parent_in_ps)
            .collect()
    };
    descendants(&running, pid)
}

fn is_terminal_on_linux(path: &str) -> bool {
    path == "/dev/ptmx"
        || path
            .strip_prefix("/dev/pts/")
            .is_some_and(|n| !n.is_empty())
}

fn is_terminal_on_macos(name: &str) -> bool {
    name == "/dev/ptmx"
        || name
            .strip_prefix("/dev/ttys")
            .is_some_and(|n| !n.is_empty())
}

fn from_proc(target: &str, unix_sockets: &str) -> Descriptor {
    let inode_of = |prefix: &str| {
        target
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_suffix(']'))
    };
    if inode_of("pipe:[").is_some() {
        return Descriptor::Pipe;
    }
    let Some(inode) = inode_of("socket:[") else {
        return if is_terminal_on_linux(target) {
            Descriptor::Terminal
        } else {
            Descriptor::File
        };
    };
    // One line for each unix socket: `Num RefCount Protocol Flags Type St Inode Path`, the path
    // only when bound.
    unix_sockets
        .lines()
        .find_map(|line| {
            let mut fields = line.split_whitespace();
            let flags = u32::from_str_radix(fields.nth(3)?, 16).ok()?;
            if fields.nth(2)? != inode {
                return None;
            }
            Some(Descriptor::UnixSocket {
                path: fields
                    .next()
                    .filter(|path| path.starts_with('/'))
                    .map(PathBuf::from),
                listens: Some(flags & 0x1_0000 != 0),
            })
        })
        .unwrap_or_else(|| Descriptor::OtherSocket("not of the unix kind".to_owned()))
}

fn from_lsof(listing: &str) -> Vec<Descriptor> {
    const NO_SOCKET: [&str; 10] = [
        "REG", "DIR", "CHR", "BLK", "LINK", "KQUEUE", "PSXSHM", "PSXSEM", "FSEVENTS", "NPOLICY",
    ];
    let mut found = Vec::new();
    let mut kind = None;
    for line in listing.lines() {
        match line.split_at_checked(1) {
            Some(("t", value)) => kind = Some(value),
            Some(("n", name)) => {
                found.push(match kind.take() {
                    Some("unix") => Descriptor::UnixSocket {
                        path: name.starts_with('/').then(|| PathBuf::from(name)),
                        listens: None,
                    },
                    Some("PIPE" | "FIFO") => Descriptor::Pipe,
                    Some("systm") => Descriptor::KernelSocket(name.to_owned()),
                    Some("CHR") if is_terminal_on_macos(name) => Descriptor::Terminal,
                    Some(file) if NO_SOCKET.contains(&file) => Descriptor::File,
                    Some(other) => Descriptor::OtherSocket(other.to_owned()),
                    None => Descriptor::OtherSocket("of no stated type".to_owned()),
                });
            }
            _ => {}
        }
    }
    found
}

/// The name of the program stands between brackets and may hold anything, so the fields are counted
/// from the last `)`.
fn parent_in_stat(stat: &str) -> Option<u32> {
    let (_, rest) = stat.rsplit_once(')')?;
    let mut fields = rest.split_whitespace();
    let state = fields.next()?;
    let parent = fields.next()?.parse().ok()?;
    (state != "Z").then_some(parent)
}

/// `Z` is a process that has ended, `E` one trying to exit. macOS lists such a one under its parent
/// while a process starts and for a moment after one ends: it runs nothing.
fn parent_in_ps(line: &str) -> Option<(u32, u32)> {
    let mut fields = line.split_whitespace();
    let pid = fields.next()?.parse().ok()?;
    let parent = fields.next()?.parse().ok()?;
    let state = fields.next()?;
    let ending = state.starts_with('Z') || state.chars().skip(1).any(|more| more == 'E');
    (!ending).then_some((pid, parent))
}

fn descendants(running: &[(u32, u32)], ancestor: u32) -> Vec<u32> {
    let mut found = BTreeSet::new();
    let mut parents = vec![ancestor];
    while let Some(parent) = parents.pop() {
        for (pid, _) in running.iter().filter(|(_, of)| *of == parent) {
            if found.insert(*pid) {
                parents.push(*pid);
            }
        }
    }
    found.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unix(path: Option<&str>, listens: Option<bool>) -> Descriptor {
        Descriptor::UnixSocket {
            path: path.map(PathBuf::from),
            listens,
        }
    }

    #[test]
    fn a_listing_of_lsof_is_read_descriptor_by_descriptor() {
        let listing = "p5564\n\
            fcwd\ntDIR\nn/private/tmp\n\
            ftxt\ntREG\nn/usr/lib/dyld\n\
            f0\ntCHR\nn/dev/null\n\
            f3\ntunix\nn/tmp/w/keeper.sock\n\
            f4\ntunix\nn->0x675bd64514cc206f\n\
            f6\ntIPv4\nPTCP\nnlocalhost:53472\n\
            f7\ntIPv6\nn*:*\n\
            f8\ntsystm\nn[ctl com.apple.netsrc id 9 unit 34]\n\
            f10\ntPIPE\nn->0xe62dd66e63945fc5\n\
            f11\ntKQUEUE\nncount=0, state=0\n\
            f12\ntCHR\nn/dev/ptmx\n\
            f13\ntCHR\nn/dev/ttys004\n\
            f14\ntCHR\nn/dev/tty\n\
            f15\ntCHR\nn/dev/ttys\n";
        assert_eq!(
            from_lsof(listing),
            [
                Descriptor::File,
                Descriptor::File,
                Descriptor::File,
                unix(Some("/tmp/w/keeper.sock"), None),
                unix(None, None),
                Descriptor::OtherSocket("IPv4".to_owned()),
                Descriptor::OtherSocket("IPv6".to_owned()),
                Descriptor::KernelSocket("[ctl com.apple.netsrc id 9 unit 34]".to_owned()),
                Descriptor::Pipe,
                Descriptor::File,
                Descriptor::Terminal,
                Descriptor::Terminal,
                Descriptor::File,
                Descriptor::File,
            ]
        );
        assert_eq!(from_lsof(""), []);
    }

    #[test]
    fn a_descriptor_of_proc_is_a_socket_of_the_kind_the_list_of_unix_sockets_gives_it() {
        let unix_sockets = "\
Num       RefCount Protocol Flags    Type St Inode Path
0000000000000000: 00000002 00000000 00010000 0001 01 23456 /tmp/w/keeper.sock
0000000000000000: 00000003 00000000 00000000 0001 03 23460 /tmp/w/keeper.sock
0000000000000000: 00000003 00000000 00000000 0001 03 23459
0000000000000000: 00000002 00000000 00010000 0001 01  1234 @/tmp/.X11-unix/X0
";
        let read = |target| from_proc(target, unix_sockets);
        assert_eq!(
            read("socket:[23456]"),
            unix(Some("/tmp/w/keeper.sock"), Some(true))
        );
        assert_eq!(
            read("socket:[23460]"),
            unix(Some("/tmp/w/keeper.sock"), Some(false))
        );
        assert_eq!(read("socket:[23459]"), unix(None, Some(false)));
        assert_eq!(read("socket:[1234]"), unix(None, Some(true)));
        assert_eq!(
            read("socket:[777]"),
            Descriptor::OtherSocket("not of the unix kind".to_owned())
        );
        assert_eq!(read("pipe:[4242]"), Descriptor::Pipe);
        assert_eq!(read("/dev/null"), Descriptor::File);
        assert_eq!(read("/dev/ptmx"), Descriptor::Terminal);
        assert_eq!(read("/dev/pts/3"), Descriptor::Terminal);
        assert_eq!(read("/dev/pts"), Descriptor::File);
        assert_eq!(read("/dev/ttyS0"), Descriptor::File);
        assert_eq!(read("anon_inode:[eventpoll]"), Descriptor::File);
    }

    #[test]
    fn the_parent_of_a_process_is_read_past_a_name_that_holds_brackets_and_spaces() {
        assert_eq!(
            parent_in_stat("812 (git) S 800 812 800 0 -1 4194304"),
            Some(800)
        );
        assert_eq!(
            parent_in_stat("812 (a (odd) name) R 800 812 800 0 -1"),
            Some(800)
        );
        assert_eq!(parent_in_stat("812 (git) Z 800 812 800 0 -1"), None);
        assert_eq!(parent_in_ps("  812   800 S+  "), Some((812, 800)));
        assert_eq!(parent_in_ps("  812   800 Z"), None);
        assert_eq!(parent_in_ps("  812   800 ?NE"), None);
        assert_eq!(parent_in_ps("  812   800 SNs"), Some((812, 800)));
    }

    #[test]
    fn the_descendants_of_a_process_are_its_children_and_theirs() {
        let running = [(2, 1), (10, 2), (11, 10), (12, 11), (20, 2), (30, 1)];
        assert_eq!(descendants(&running, 10), [11, 12]);
        assert_eq!(descendants(&running, 2), [10, 11, 12, 20]);
        assert_eq!(descendants(&running, 12), Vec::<u32>::new());
    }
}
