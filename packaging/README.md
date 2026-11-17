# Packaging

| Abbreviation | Meaning |
| --- | --- |
| Window, Keeper | The two programs: `yaar-review` draws, `yaar-keeper` runs behind it |
| apt repository | A folder of packages with a signed index, from which `apt` installs and updates |
| VM | The Ubuntu 24.04 arm64 virtual machine the Linux checks ran in |

## Files

| File | What it does |
| --- | --- |
| `package-linux.sh` | Makes the `.deb` and the plain archive from two built programs |
| `linux/io.github.tsadoq.yaar.desktop`, `linux/icon-256.png` | Desktop entry and icon that go into both. The icon is a placeholder |
| `apt/make-apt-repo.sh` | Builds or refreshes a signed apt repository of static files |
| `apt/make-test-key.sh` | Makes a throwaway signing key for CI and tests. Never the key of a release |
| `install.sh` | The script behind `curl -fsSL <site>/install.sh \| bash` |
| `package-macos.sh`, `macos/`, `notices.sh`, `notices.hbs` | The Mac app and the third-party notices. Each explains itself in its header |

## The Linux packages

Run on the system the programs were built on, which for a release is Ubuntu 22.04: a
program built on a newer release does not start on an older one.

```sh
.github/scripts/release-build.sh
packaging/notices.sh dist/THIRD-PARTY-NOTICES.txt "$(uname -m)-unknown-linux-gnu"
NOTICES=dist/THIRD-PARTY-NOTICES.txt packaging/package-linux.sh \
  target/release/yaar-review target/release/yaar-keeper 0.1.0 dist
```

It writes `yaar-review_<version>_<arm64|amd64>.deb`,
`yaar-review-<version>-linux-<aarch64|x86_64>.tar.gz` and `SHA256SUMS`. Two runs on the
same programs give the same bytes. Needs `dpkg-dev`. On Ubuntu 22.04 the build itself also
needs `g++`: without it `freetype-sys` stops with `failed to find tool "c++"`.

A version may hold letters, digits, `.`, `+` and `~`. A `-` is refused, because apt would
sort `0.2.0-rc1` after `0.2.0`; write `0.2.0~rc1`.

| Installed by the `.deb` | |
| --- | --- |
| `/usr/bin/yaar-review` | The Window |
| `/usr/libexec/yaar-review/yaar-keeper` | The Keeper. The Window looks for it at `../libexec/yaar-review/` from its own place |
| `/usr/share/applications/io.github.tsadoq.yaar.desktop` | |
| `/usr/share/icons/hicolor/256x256/apps/io.github.tsadoq.yaar.png` | |
| `/usr/share/doc/yaar-review/` | `copyright`, `NOTICE`, `THIRD-PARTY-NOTICES.txt.gz` |

The archive holds the same tree under `yaar-review/`, with `LICENSE` added.

`dpkg-shlibdeps` writes the libraries the programs link into `Depends`. The rest is
written by hand, because the program opens these at run time or runs them. Each was found
by adding it after a failure, in an `ubuntu:24.04` container that held only the archive:

| Added | Without it |
| --- | --- |
| `libxcb1`, `libxkbcommon0`, `libxkbcommon-x11-0` (found by `dpkg-shlibdeps`) | `error while loading shared libraries: libxcb.so.1` |
| `libwayland-client0` | `failed to connect to Wayland compositor: The wayland library could not be loaded` |
| `libvulkan1` and `mesa-vulkan-drivers \| vulkan-icd` | `Failed to create surface for any enabled backend` |
| `fonts-dejavu-core` | `failed to resolve font '.SystemUIFont' or any of the fallbacks` |
| `git` | The window draws. It is a dependency because the app runs it |

`libegl1` and `libwayland-egl1` are not listed. With them and no Vulkan driver the window
drew under Wayland and did not open under X11.

## The apt repository

```sh
export GNUPGHOME=<folder with the signing key> YAAR_APT_KEY=<its fingerprint>
packaging/apt/make-apt-repo.sh site/apt dist/yaar-review_0.1.0_arm64.deb dist/yaar-review_0.1.0_amd64.deb
```

Each `.deb` is copied into `pool/`; the index of suite `stable`, component `main` is
rebuilt from everything in `pool/`, one `binary-<arch>` folder per architecture found, and
signed. The public key is written beside it as `yaar-review.asc`. Needs `apt-utils` and
`gnupg`.

For a test, with a key that lasts seven days:

```sh
eval "$(packaging/apt/make-test-key.sh)"; export GNUPGHOME YAAR_APT_KEY
packaging/apt/make-apt-repo.sh site/apt dist/*.deb
gpgconf --homedir "$GNUPGHOME" --kill gpg-agent; rm -rf "$GNUPGHOME"
```

The key of the published repository does not exist yet.

## The install script

```sh
curl -fsSL https://tsadoq.github.io/yaar/install.sh | bash
curl -fsSL https://tsadoq.github.io/yaar/install.sh | bash -s -- --uninstall
curl -fsSL https://tsadoq.github.io/yaar/install.sh | bash -s -- --dry-run
```

| System | What it does |
| --- | --- |
| Debian, Ubuntu and systems built on them | Saves the key to `/etc/apt/keyrings/yaar-review.asc`, writes `/etc/apt/sources.list.d/yaar-review.sources` with `Signed-By`, runs `apt-get update` and `apt-get install yaar-review`. Uses `sudo` when not root. If apt refuses, the key and the source file are put back as they were |
| Other Linux | Takes the archive named in `SHA256SUMS`, checks its SHA-256, copies it under `~/.local`. Installs no library |
| macOS | Takes `yaar-review-<version>-macos.zip`, checks its SHA-256, verifies the code signature of `YAAR.app`, copies it to `/Applications`, or to `~/Applications` when that is not writable. Never starts it |

The whole body is one function called on the last line, so a download cut short runs
nothing.

| Variable | Default |
| --- | --- |
| `YAAR_BASE_URL` | `https://tsadoq.github.io/yaar` |
| `YAAR_APT_URL` | `$YAAR_BASE_URL/apt` |
| `YAAR_RELEASES_URL` | `https://github.com/Tsadoq/yaar/releases` |
| `YAAR_DOWNLOAD_URL` | `$YAAR_RELEASES_URL/latest/download` |
| `YAAR_METHOD` | `auto`; `apt` or `archive` forces a route on Linux |
| `YAAR_PREFIX` | `~/.local`, target of the archive route |
| `YAAR_INSTALL_DIR` | Target folder of the Mac app |

A release therefore publishes, on the site: `install.sh` and `apt/`. As assets of the
release: the archives, the `.zip`, and one `SHA256SUMS` that names them all (the Linux
and the macOS packaging each write their own list; the release job joins them).

## What was checked, and where

On 2026-11-17, on arm64, in containers inside the VM, with everything served from a local
web server. Nothing was fetched from the two real addresses. The containers ran with an
init as their first process, so that a Keeper that has left is seen as gone.

| Check | Result |
| --- | --- |
| Programs built in `ubuntu:22.04`, packaged there | They ask for glibc 2.35 at most |
| `.deb` installed with apt on Ubuntu 22.04, 24.04, 26.04, Debian 12, 13 | Installs. A user who is not root starts the Keeper from `/usr/libexec/yaar-review/`; a second call finds the same one; it leaves by itself 5 s after the last call |
| The window under a headless Wayland compositor (sway), software Vulkan | Draws "YAAR" and "Keeper 0.1.0 is running" on all five, also when installed with `--no-install-recommends` |
| The window under X11 (Xwayland, Xvfb), software Vulkan | Draws on Ubuntu 24.04, 26.04 and Debian 13. On Ubuntu 22.04 and Debian 12 the window is created and stays empty |
| Repository built with a test key; `install.sh` piped from the local server | Installs from the repository on all five, as root and as a user with `sudo`; a second run changes nothing; `--uninstall` removes the package, the key, the source file and apt's copy of the index. What stays is the Keeper's own folder in the home of each user who ran it, `~/.local/share/yaar-review`, with an empty `keeper.sock.lock` |
| The `.deb` changed by one byte after signing | apt: `Hash Sum mismatch`, exit 100. The script then exits 1 and leaves no key and no source file |
| Script cut off, archive with a wrong checksum, no `sudo`, unknown processor | Each exits with an error and changes nothing |
| Archive route on Fedora 45, as a user who is not root | Installs under `~/.local`; after `dnf install libxcb libxkbcommon libxkbcommon-x11` both programs run |
| macOS route, on the Mac, into a temporary folder, with the ad hoc signed app | Installs, replaces, removes; a wrong checksum and a cut-off script change nothing |
| `shellcheck` 0.9.0, `desktop-file-validate` | No finding |

## Not checked, and open

- An x86-64 package. The repository builder was run on two architectures with a stand-in:
  the arm64 package with its `Architecture` field rewritten.
- A real desktop session, a hardware Vulkan driver, a launcher showing the entry and icon.
- X11 on Ubuntu 22.04 and Debian 12 with a hardware driver. With Mesa's software Vulkan
  of those releases the window stayed empty; the cause is not isolated.
- The window's app id, `io.github.tsadoq.yaar`, was set in the code after these runs. A
  throwaway build with the same line reported it under Wayland and as `WM_CLASS` under
  X11; the packages built from the committed code were not looked at again.
- GitHub Pages and a release's download address: HTTPS, redirects, caching.
- An upgrade from one published version to the next.
- `Maintainer` in the `.deb` carries the no-reply address of the repository owner's GitHub
  account. Whether a real contact address should stand there is the owner's call.
- The programs are not stripped (`lintian`: `unstripped-binary-or-object`), and the
  package has no changelog and no manual page.
- Ubuntu's container images drop everything under `/usr/share/doc` except `copyright`
  and changelogs (`path-exclude` in their dpkg configuration), so the notices are in the
  package and absent after an install there. Debian's images keep them.
