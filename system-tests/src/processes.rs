//! What a process holds open and which processes it started, read from outside it.
//!
//! On Linux both are read from `/proc`, which a plain container has. Elsewhere they are
//! asked of `lsof` and `ps`, which macOS has.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// One thing a process holds open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Descriptor {
    /// A socket of the unix kind: one that leads nowhere beyond this machine.
    UnixSocket {
        /// The file it is bound at, when it is bound at one. A connection that a
        /// listener accepted carries the path of the listener.
        path: Option<PathBuf>,
        /// Whether it listens, where the system says so: Linux does, `lsof` does not.
        listens: Option<bool>,
    },
    /// A socket of any other kind, with what the system calls it. Whatever the listing
    /// cannot place counts as one, so that a kind nobody thought of fails a test and does
    /// not pass it.
    OtherSocket(String),
    /// A socket of macOS's system domain, with the name `lsof` gives it: a line from the
    /// process to its own kernel, as the system's libraries open for themselves in any
    /// process. No other process and no network can connect to one.
    KernelSocket(String),
    /// One end of a pipe.
    Pipe,
    /// A terminal device: the end of a pseudo-terminal the Keeper holds, or the terminal a
    /// program runs on. Neither is a socket.
    Terminal,
    /// A file, a folder, a device that is no terminal, or anything else that is no
    /// socket and no pipe.
    File,
}

/// Everything the process holds open, in one listing. Empty when the process is not
/// there or cannot be read: an empty listing says nothing about a process.
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
        // No selector but the process: every descriptor of every kind is listed.
        let listed = Command::new("lsof")
            .args(["-a", "-p", &pid.to_string(), "-F", "ftn"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .expect("start lsof");
        from_lsof(&String::from_utf8_lossy(&listed.stdout))
    }
}

/// Every process that descends from this one and still runs: its children, theirs, and
/// so on. One that has ended and waits to be collected holds nothing open and is left
/// out, and so is one that is on its way out.
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

/// What a link in `/proc/<pid>/fd` leads to, with the content of `/proc/net/unix` to
/// tell the kind of a socket.
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
        return Descriptor::File;
    };
    // One line for each unix socket of the machine:
    // `Num RefCount Protocol Flags Type St Inode Path`, the path only when bound.
    unix_sockets
        .lines()
        .find_map(|line| {
            let mut fields = line.split_whitespace();
            let flags = u32::from_str_radix(fields.nth(3)?, 16).ok()?;
            if fields.nth(2)? != inode {
                return None;
            }
            Some(Descriptor::UnixSocket {
                // A name that starts with `@` is no file.
                path: fields
                    .next()
                    .filter(|path| path.starts_with('/'))
                    .map(PathBuf::from),
                // The flag a socket carries while it accepts connections.
                listens: Some(flags & 0x1_0000 != 0),
            })
        })
        .unwrap_or_else(|| Descriptor::OtherSocket("not of the unix kind".to_owned()))
}

/// What `lsof -F ftn` printed for one process: for each descriptor a line `f` with its
/// number, a line `t` with its type and a line `n` with its name.
fn from_lsof(listing: &str) -> Vec<Descriptor> {
    // The types `lsof` gives to what is no socket. Any other type counts as a socket.
    const NO_SOCKET: [&str; 10] = [
        "REG", "DIR", "CHR", "BLK", "LINK", "KQUEUE", "PSXSHM", "PSXSEM", "FSEVENTS", "NPOLICY",
    ];
    let mut found = Vec::new();
    let mut kind = None;
    for line in listing.lines() {
        match line.split_at_checked(1) {
            // The name closes the lines of one descriptor.
            Some(("t", value)) => kind = Some(value),
            Some(("n", name)) => {
                found.push(match kind.take() {
                    Some("unix") => Descriptor::UnixSocket {
                        // A socket with no file shows the address of the other end.
                        path: name.starts_with('/').then(|| PathBuf::from(name)),
                        listens: None,
                    },
                    Some("PIPE" | "FIFO") => Descriptor::Pipe,
                    Some("systm") => Descriptor::KernelSocket(name.to_owned()),
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

/// The parent of the process in the one line of its `/proc/<pid>/stat`; none for a
/// process that has ended. The name of the program stands between brackets and may hold anything, so
/// the fields are counted from the last `)`: the state, then the parent.
fn parent_in_stat(stat: &str) -> Option<u32> {
    let (_, rest) = stat.rsplit_once(')')?;
    let mut fields = rest.split_whitespace();
    let state = fields.next()?;
    let parent = fields.next()?.parse().ok()?;
    (state != "Z").then_some(parent)
}

/// The process and its parent in one line of `ps -o pid=,ppid=,stat=`; none for a
/// process that has ended or is ending.
///
/// The state is one letter for what the process does, `Z` when it has ended, and then
/// letters that say more, `E` among them for one that is trying to exit. While a process
/// is started, and for a moment after one has ended, macOS lists such a one under its
/// parent: it runs nothing and is about to be gone.
fn parent_in_ps(line: &str) -> Option<(u32, u32)> {
    let mut fields = line.split_whitespace();
    let pid = fields.next()?.parse().ok()?;
    let parent = fields.next()?.parse().ok()?;
    let state = fields.next()?;
    let ending = state.starts_with('Z') || state.chars().skip(1).any(|more| more == 'E');
    (!ending).then_some((pid, parent))
}

/// Those of `running`, given as pairs of a process and its parent, that descend from
/// `ancestor`.
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
            f11\ntKQUEUE\nncount=0, state=0\n";
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
        // A socket the list of unix sockets lacks is of another kind: a network port.
        assert_eq!(
            read("socket:[777]"),
            Descriptor::OtherSocket("not of the unix kind".to_owned())
        );
        assert_eq!(read("pipe:[4242]"), Descriptor::Pipe);
        assert_eq!(read("/dev/null"), Descriptor::File);
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
