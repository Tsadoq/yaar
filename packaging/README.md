# Packaging

| Abbreviation | Meaning |
| --- | --- |
| Window, Keeper | The two programs: `yaar-review` draws, `yaar-keeper` runs behind it |
| the five programs | `yaar-review`, `yaar-keeper`, `yaar-claude-hook`, `yaar-claude-tools`, `yaar-pi-report`: what every package holds |
| apt repository | A folder of packages with a signed index, from which `apt` installs and updates |
| site | What GitHub Pages serves: the page, `install.sh` and the apt repository |
| release files | The six packages, `SHA256SUMS` and its signature, which a release holds |
| base address | The address of the site: the one line of `site/base-url` |
| rehearsal | A run of the workflow that publishes nothing; every run but the owner's `publish` |
| VM | The Ubuntu 24.04 arm64 virtual machine the Linux checks ran in |

## Files

| File | What it does |
| --- | --- |
| `version.sh` | Prints the one version number, the `version` of `[workspace.package]` in `Cargo.toml` |
| `package-linux.sh` | Makes the `.deb` and the plain archive from the five built programs |
| `linux/io.github.tsadoq.yaar.desktop`, `linux/icon-256.png` | Desktop entry and icon that go into both. The icon is a placeholder |
| `package-macos.sh`, `macos/` | Make `YAAR.app`, its `.zip` and its `.dmg`; `macos/` holds `Info.plist.in`, the icon and `team-id`, the Team ID of the signer |
| `notices.sh`, `notices.hbs`, `notices-count.sh` | Write the third-party notices that go into every package, and count them against the lock file |
| `apt/make-apt-repo.sh` | Builds or refreshes a signed apt repository of static files |
| `apt/make-test-key.sh` | Makes a throwaway signing key for CI and tests. Never the key of a release |
| `apt/repack-version.sh` | The same `.deb` at another version, for the rehearsal's later version |
| `apt/project-key.fingerprint` | The fingerprint of the project's key, which the owner writes once |
| `release/assemble.sh` | Makes the release files and the site |
| `release/later.sh` | Makes the rehearsal's second site, with a later version |
| `release/notes.md.in` | The release notes, with `@VERSION@` and `@BASE_URL@` |
| `site/index.html.in`, `site/base-url` | The site's page, and the one line that is the base address |
| `install.sh` | The install script of the site |
| `release-day.md` | The README's section "Install" as it reads once a release is published |
| `checklists/` | The owner's scripts for the checks that are made by hand; its README names one command for each item |

The scripts of the workflow's release steps are in `.github/scripts/`; "The release files and
the site" below says which step calls which.

## The Linux packages

Run on the system the programs were built on, which for a release is Ubuntu 22.04: a
program built on a newer release does not start on an older one.

```sh
.github/scripts/release-build.sh
packaging/notices.sh dist/THIRD-PARTY-NOTICES.txt "$(uname -m)-unknown-linux-gnu"
NOTICES=dist/THIRD-PARTY-NOTICES.txt packaging/package-linux.sh \
  target/release "$(packaging/version.sh)" dist
```

`target/release` holds the five programs. It writes `yaar-review_<version>_<arm64|amd64>.deb`
and `yaar-review-<version>-linux-<aarch64|x86_64>.tar.gz`, and no checksum file: the release
has one, written by `release/assemble.sh`. Two runs on the same programs give the same bytes.
Needs `dpkg-dev`. On Ubuntu 22.04 the build itself also needs `g++`: without it `freetype-sys`
stops with `failed to find tool "c++"`.

The version is the one of `version.sh`: three numbers, `X.Y.Z`. A `-` is refused in a version,
because apt would sort `0.2.0-rc1` after `0.2.0`.

| Installed by the `.deb` | |
| --- | --- |
| `/usr/bin/yaar-review` | The Window |
| `/usr/libexec/yaar-review/yaar-keeper` | The Keeper. The Window looks for it at `../libexec/yaar-review/` from its own place |
| `/usr/libexec/yaar-review/yaar-claude-hook`, `yaar-claude-tools`, `yaar-pi-report` | Claude Code's hook program and tool server, and pi's reporting program, which the Keeper finds beside itself |
| `/usr/share/applications/io.github.tsadoq.yaar.desktop` | |
| `/usr/share/icons/hicolor/256x256/apps/io.github.tsadoq.yaar.png` | |
| `/usr/share/doc/yaar-review/` | `copyright`, `NOTICE`, `THIRD-PARTY-NOTICES.txt.gz` |

The archive holds the same tree under `yaar-review/`, with `LICENSE` added and the notices not
compressed.

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

## The Mac app

```sh
.github/scripts/release-build.sh --target aarch64-apple-darwin
.github/scripts/release-build.sh --target x86_64-apple-darwin
packaging/notices.sh dist/THIRD-PARTY-NOTICES.txt aarch64-apple-darwin x86_64-apple-darwin
NOTICES=dist/THIRD-PARTY-NOTICES.txt packaging/package-macos.sh "$(packaging/version.sh)" dist \
  target/aarch64-apple-darwin/release target/x86_64-apple-darwin/release
```

With one programs folder the app holds that processor's programs; with two it holds universal
ones, made with `lipo`. It writes `YAAR.app`, a `.zip` and a `.dmg` into `dist`, named
`yaar-review-<version>-macos-universal` with two programs folders and
`yaar-review-<version>-macos-arm64` or `-macos-x86_64` with one (the release holds the
universal pair). The Keeper and the three programs beside it lie in `Contents/Helpers/`. The app is signed ad hoc unless
`SIGN_IDENTITY` names a Developer ID Application identity: an ad hoc app runs on the Mac that
made it, and another Mac's Gatekeeper refuses it. Only the workflow's job
`release-sign-macos` signs with the Developer ID and has Apple notarise the app and the disk
image; the header of `package-macos.sh` says how, and its log says which signature it made, in
one line starting `signature: `.

## The apt repository

```sh
export GNUPGHOME=<folder with the signing key> YAAR_APT_KEY=<its fingerprint>
packaging/apt/make-apt-repo.sh site/apt dist/yaar-review_*_arm64.deb dist/yaar-review_*_amd64.deb
```

Each `.deb` is copied into `pool/`; the index of suite `stable`, component `main` is
rebuilt from everything in `pool/`, one `binary-<arch>` folder per architecture found, and
signed. The public key is written beside it as `yaar-review.asc`. A file of the pool that the
index would leave out is refused, naming it. Needs `apt-utils` and `gnupg`.

For a test, with a key that lasts seven days:

```sh
eval "$(packaging/apt/make-test-key.sh)"; export GNUPGHOME YAAR_APT_KEY
packaging/apt/make-apt-repo.sh site/apt dist/*.deb
gpgconf --homedir "$GNUPGHOME" --kill gpg-agent; rm -rf "$GNUPGHOME"
```

The key of the published repository is the project's key, made once by the owner (below): its
fingerprint is the line of `apt/project-key.fingerprint`, and the job that signs a release
refuses any other key.

## The release files and the site

`release/assemble.sh <version> <packages-dir> <out-dir>` makes a release as files, with the
key `GNUPGHOME` and `YAAR_APT_KEY` name: the rehearsal's throwaway key in the job `assemble`,
the project's key in the job `assemble-release`. `PREVIOUS=<folder>` names the `.deb` files
of the releases already published, which go into the pool beside the new ones.

| Written | What it is |
| --- | --- |
| `release/` | The six packages, `SHA256SUMS` (one line for each, sorted by name) and `SHA256SUMS.asc`, its signature by the key |
| `site/index.html` | The page of `site/index.html.in`: what the app is, the install line, the manual routes, the key's fingerprint |
| `site/install.sh` | `install.sh` with its base address and the Team ID of `macos/team-id` put in |
| `site/apt/` | The apt repository of the new `.deb` files and those of `PREVIOUS`, and the public key `yaar-review.asc` |
| `key.asc` | The public key beside the two folders, for the tests |

It refuses a package that is missing or that holds another version or processor than its name
says, and a site of more than 900 MB (a Pages site may hold 1 GB; the pool grows by two files
a release), naming the versions in the pool. `release/later.sh` makes, beside a rehearsal's
`site`, `site-later`, whose repository also holds both `.deb` files repacked at a later
version; it is never part of a release.

The release job calls these scripts of `.github/scripts/`, each by its step:

| Step | Script | What it refuses |
| --- | --- | --- |
| The project's key, checked against the fingerprint in the repository; The project's key is removed | `project-key.sh import`, `remove` | An empty `APT_SIGNING_KEY`, a file with no fingerprint line, a key of another fingerprint. `import` makes `$RUNNER_TEMP/yaar-apt-key` as `GNUPGHOME` and removes it again when it refuses |
| The packages already published | `fetch-published-debs.sh` | A `.deb` of an earlier release whose SHA-256 is not the one that release's `SHA256SUMS` names, and a failed list of releases |
| One checksum file lists every package of the release | `release-files-check.sh` | A file that is not one of the eight, a sum that does not hold, an unsorted list, a signature that does not verify |
| The index and the checksums verify with the key the site publishes | `verify-signed.sh` | A signature that does not verify with the key the site publishes alone, a key of another fingerprint, a hash that `InRelease` does not name, a `.deb` that is not what `Packages` says, an `install.sh` that holds `@BASE_URL@` or `@TEAM_ID@` or another base address or team |
| A draft release with every file; The release, published as the latest; The release is tagged on the public repository, and its source holds the licence | `publish.sh draft`, `publish`, `check` | See the day of a release, below |
| The texts name the one base address | `base-url-check.sh` | An address in a text that is not the base address |

`verify-signed.sh <final-dir>` and `release-files-check.sh <release-dir> <version>` can be
run by hand on what `assemble.sh` wrote. `fetch-published-debs.sh`, `publish.sh` and
`project-key.sh` need GitHub or the project's key and run in the job only.

Once the owner has approved the run, the job `publish` makes the draft release with the eight
files and the notes of `release/notes.md.in` (`publish.sh draft`), deploys the site, makes the
release the latest and waits until `releases/latest/download/SHA256SUMS` answers with the new
checksum file (`publish.sh publish`), and then checks that the tag on the public repository is
the commit of the run, that the source archive GitHub made for the tag holds `LICENSE`, and
that the site answers with the new `install.sh`, `InRelease` and key (`publish.sh check`,
which waits about ten minutes in all for Pages' cache). A published release of that tag with
other files is refused: a published version is never changed.

### The base address

The base address is the one line of `site/base-url`. `assemble.sh` writes it into the site's
`install.sh` and `index.html`, and `publish.sh` into the release notes, from that file alone.
The two texts that must show it as it is, `README.md` (once the release's pull request has
copied `release-day.md` into it) and this file, hold copies, and the step "The texts name the
one base address" (`.github/scripts/base-url-check.sh`) fails on any address that ends in
`/install.sh` or `/apt` and does not start with the line of `site/base-url` (the line itself,
or the line followed by `/`), is not on `127.0.0.1` or `localhost`, and is not the placeholder
`@BASE_URL@`, naming its file, line and address. `release-day.md` holds the placeholder, so that the
release's pull request puts the line in.

## The install script

`install.sh` on the site is `packaging/install.sh` with its base address and Team ID put in.
`release-day.md` holds the lines a user types. Here the script is run against a local server,
on the files a rehearsal made (the artifact `rehearsal` of a run, unpacked into `rehearsal/`):

```sh
.github/scripts/serve-site.sh start rehearsal/site rehearsal/release 8000
YAAR_BASE_URL=http://127.0.0.1:8000 YAAR_DOWNLOAD_URL=http://127.0.0.1:8000/download bash packaging/install.sh
.github/scripts/serve-site.sh stop 8000
```

The script takes `--dry-run`, which says what it would do and changes nothing, and
`--uninstall`, with `--purge` and `--stop-agents`, after the same two variables. A rehearsal's
Mac app is signed ad hoc: from a loopback download address `YAAR_ALLOW_UNNOTARIZED=1` makes
the script check its signature and nothing else, and with any other address it refuses.

| System | What it does |
| --- | --- |
| Debian, Ubuntu and systems built on them | Saves the key to `/etc/apt/keyrings/yaar-review.asc`, writes `/etc/apt/sources.list.d/yaar-review.sources` with `Signed-By`, runs `apt-get update` and `apt-get install yaar-review`. Uses `sudo` when not root. Each of the two files is written only when its bytes differ, so a second run changes nothing. If apt refuses, the key and the source file are put back as they were |
| Other Linux | Takes the archive named in `SHA256SUMS`, checks its SHA-256, copies it under `~/.local`. Installs no library |
| macOS | Takes `yaar-review-<version>-macos-universal.zip`, checks its SHA-256, then the app: its code signature, that the certificate that signed it is of the Team ID the script names, Gatekeeper's verdict and the stapled ticket. Copies it to `/Applications`, or to `~/Applications` when that is not writable. Never starts it |

The uninstall removes the app wherever it is (the package, the files of the archive route, the
bundle) and the app's own files in the cache, runtime and data folders, and the one short
socket folder of the user's own Keeper. It keeps the user's reviews (`store.db` and `states/`
of the data folder) and pi's extension (`agents/pi/`); `--purge` removes the data folder
whole. It stops and says so while the app runs, since agents may be running in its terminals,
unless `--stop-agents` is given, which ends the app and them. It touches nothing of the user's
repositories and nothing of pi's or Claude Code's own settings.

The whole body is one function called on the last line, so a download cut short runs
nothing.

| Variable | Default |
| --- | --- |
| `YAAR_BASE_URL` | The line of `site/base-url`, which `assemble.sh` puts into the site's copy; the copy in the repository needs it set |
| `YAAR_APT_URL` | `$YAAR_BASE_URL/apt` |
| `YAAR_RELEASES_URL` | `https://github.com/Tsadoq/yaar/releases` |
| `YAAR_DOWNLOAD_URL` | `$YAAR_RELEASES_URL/latest/download` |
| `YAAR_METHOD` | `auto`; `apt` or `archive` forces a route on Linux |
| `YAAR_PREFIX` | `~/.local`, target of the archive route |
| `YAAR_INSTALL_DIR` | Target folder of the Mac app |
| `YAAR_ALLOW_UNNOTARIZED` | `1` for a rehearsal's ad hoc app, from a loopback download address only |

A release publishes, on the site, `install.sh`, `index.html` and `apt/`, and, as assets of
the release, the eight files: the `.deb` and the plain archive of each processor, the `.zip`
and the `.dmg`, `SHA256SUMS` and `SHA256SUMS.asc`.

## The release, and what the owner does once

A release is the workflow `ci` run by hand on a version tag with the input `publish` on.
Every other run of it, a pull request's, a push's or a tag's, is a rehearsal: it builds,
signs the app ad hoc and the apt repository with a key it makes and throws away, makes the
checksums, the site and the repository as files, tests them, and keeps them as the artifact
`rehearsal` of the run. It publishes nothing and sends nothing to Apple. The job `version`
decides which of the two a run is, in its step "What this run is", and says so in its log.

A run that publishes signs the app with the Developer ID in a keychain of its own on
GitHub's Mac, has Apple notarise the app and the disk image, staples both, checks them with
`codesign`, `spctl` and `stapler`, removes the keychain; signs the apt repository and the
checksum file with the project's key; and then waits for the owner's approval. Once he has
walked the release's checklists on those very files and approved, it makes the GitHub
release (a draft first), deploys the Pages site, and then publishes the release as the
latest.

### The secrets, and two public files

Six secrets of the environment `release`, which only the jobs `release-sign-macos` and
`assemble-release` name, and which the environment gives to runs on the tags `v*` only. A
run of a pull request or of a branch never sees them.

| Secret | What it holds | Form |
| --- | --- | --- |
| `MACOS_CERTIFICATE_P12` | The Developer ID Application certificate with its private key, and Apple's Developer ID Certification Authority (G2) certificate | The `.p12` file the two were exported into together, in base64 |
| `MACOS_CERTIFICATE_PASSWORD` | The password the `.p12` was exported with | Text |
| `APPLE_NOTARY_KEY_P8` | An App Store Connect API key, for `notarytool` | The text of the file `AuthKey_<key id>.p8` |
| `APPLE_NOTARY_KEY_ID` | That key's id | Text, ten characters |
| `APPLE_NOTARY_ISSUER_ID` | The issuer id of the team's keys | Text, a UUID |
| `APT_SIGNING_KEY` | The project's OpenPGP key that signs the apt repository and `SHA256SUMS` | The armoured secret key, ed25519, no passphrase on this copy |

Two public facts lie in the repository, each one line under its comments:
`apt/project-key.fingerprint`, the apt key's fingerprint, and `macos/team-id`, the Team ID of
the Developer ID. The job refuses an apt key whose fingerprint differs and a certificate of
another team, and the install script on the site refuses an app that another team signed. The
apt key is made once and never replaced: a client that trusts the old key meets a new one
with a warning and no error, and stops receiving updates.

### What guards a release

- The environments `release` and `github-pages` admit the tags `v*` only, and
  `github-pages` waits for the owner's approval.
- The job `version` refuses to publish a tag whose commit is not on `main`.
- A pull request from a branch of this repository runs its own copy of the workflow, which
  could ask for more permissions than `ci.yml` gives it. The environments keep the secrets
  and the site from it; nothing keeps it from writing a release with its own token. Review
  is the guard, as in any repository, and the optional step 6 below narrows who may push a
  tag `v*`.

### Before the first release, once

Each command reads a value from a file, the pasteboard or a prompt; none puts a secret on a
command line or into a log. `gh` must be logged in as the owner of `Tsadoq/yaar`. Every file
a step makes goes into one temporary folder, which macOS keeps under `/private/var/folders`,
outside the Desktop and Documents that iCloud may sync, and which the last step deletes:

```sh
D="$(mktemp -d)"; echo "$D"; open "$D"
```

1. The Pages site is built by the workflow:

   ```sh
   gh api -X POST repos/Tsadoq/yaar/pages -f build_type=workflow
   ```

2. The environment `release`, for the tags `v*` only; the same rule on the environment
   `github-pages` that step 1 made, and the owner's approval before it deploys:

   ```sh
   echo '{"deployment_branch_policy":{"protected_branches":false,"custom_branch_policies":true}}' \
     | gh api -X PUT repos/Tsadoq/yaar/environments/release --input -
   gh api -X POST repos/Tsadoq/yaar/environments/release/deployment-branch-policies -f name='v*' -f type=tag
   printf '{"reviewers":[{"type":"User","id":%s}],"deployment_branch_policy":{"protected_branches":false,"custom_branch_policies":true}}' \
     "$(gh api user --jq .id)" | gh api -X PUT repos/Tsadoq/yaar/environments/github-pages --input -
   gh api -X POST repos/Tsadoq/yaar/environments/github-pages/deployment-branch-policies -f name='v*' -f type=tag
   ```

3. The certificate. In Keychain Access select both "Developer ID Application: … (<Team
   ID>)" with its key, under My Certificates, and "Developer ID Certification Authority"
   (G2), under Certificates; File > Export Items…, format Personal Information Exchange
   (`.p12`), saved into the folder `$D` opened above as `developer-id.p12`, with a new
   password. The Team ID is the ten characters in parentheses. Then:

   ```sh
   base64 -i "$D/developer-id.p12" | gh secret set MACOS_CERTIFICATE_P12 --env release --repo Tsadoq/yaar
   gh secret set MACOS_CERTIFICATE_PASSWORD --env release --repo Tsadoq/yaar   # asks; type the password
   ```

4. The notary key. The first time, the Account Holder of the developer account must request
   access to the App Store Connect API (Users and Access > Integrations > App Store Connect
   API, "Request Access") and accept its terms. Then, on the same page, under Team Keys, add
   a key with the access Developer and download it (it can be downloaded once). Apple
   documents no smallest role for `notarytool`; Developer is reported to work, and the first
   publishing run checks the key with Apple before it signs anything, so a key that cannot
   notarise stops the run there. The page shows the Key ID and the Issuer ID. Then:

   ```sh
   mv ~/Downloads/AuthKey_<KEY ID>.p8 "$D"/
   gh secret set APPLE_NOTARY_KEY_P8 --env release --repo Tsadoq/yaar < "$D"/AuthKey_<KEY ID>.p8
   gh secret set APPLE_NOTARY_KEY_ID --env release --repo Tsadoq/yaar      # asks; paste the Key ID
   gh secret set APPLE_NOTARY_ISSUER_ID --env release --repo Tsadoq/yaar   # asks; paste the Issuer ID
   ```

   Copy the `.p8` file to where you keep keys offline, if you want one.

5. The apt key, in a checkout of the repository on a branch of its own:

   ```sh
   export GNUPGHOME="$D/gnupg"; mkdir -m 700 "$GNUPGHOME"
   gpg --batch --passphrase '' --quick-generate-key 'YAAR apt repository' ed25519 sign never
   gpg --batch --with-colons --list-secret-keys | awk -F: '$1=="fpr"{print $10; exit}' \
     >> packaging/apt/project-key.fingerprint
   gpg --batch --armor --export-secret-keys | gh secret set APT_SIGNING_KEY --env release --repo Tsadoq/yaar
   gpg --batch --armor --export-secret-keys | gpg --symmetric --armor --output "$D/yaar-apt-key.asc.gpg"   # asks for a passphrase
   gpgconf --kill gpg-agent; unset GNUPGHOME
   ```

   Copy `yaar-apt-key.asc.gpg`, the key under that passphrase, to where you keep keys
   offline. Add the Team ID of step 3 as the last line of `packaging/macos/team-id`, and
   commit both files through a pull request.

6. Optional: only the owner may push a tag `v*`:

   ```sh
   echo '{"name":"release tags","target":"tag","enforcement":"active","conditions":{"ref_name":{"include":["refs/tags/v*"],"exclude":[]}},"rules":[{"type":"creation"},{"type":"update"},{"type":"deletion"}],"bypass_actors":[{"actor_id":5,"actor_type":"RepositoryRole","bypass_mode":"always"}]}' \
     | gh api -X POST repos/Tsadoq/yaar/rulesets --input -
   ```

7. Delete the folder and everything in it:

   ```sh
   rm -rf "$D"; unset D
   ```

### The day of a release

The version in `Cargo.toml` (`[workspace.package]`) is the one number; the pull request
that raises it is merged first. Then, from the merged commit:

```sh
git tag v<version> && git push origin v<version>        # a rehearsal runs on the tag
gh workflow run ci.yml --repo Tsadoq/yaar --ref v<version> -f publish=true
```

The run refuses a tag that is not `v` and the version of `Cargo.toml`, and a tag whose commit
is not on `main`. When every job before `publish` is green, the run waits for the approval.
Meanwhile, the checklists are walked on the files the run made (`packaging/checklists/`,
whose README names a command for each item):

```sh
gh run download <run id> --repo Tsadoq/yaar -n release-final -D ~/yaar-release-<version>
packaging/checklists/serve-release.sh ~/yaar-release-<version>
```

Then approve on the run's page (Review deployments), or:

```sh
gh api -X POST repos/Tsadoq/yaar/actions/runs/<run id>/pending_deployments \
  -F "environment_ids[]=$(gh api repos/Tsadoq/yaar/environments/github-pages --jq .id)" \
  -f state=approved -f comment='checklists walked'
```

A run left waiting is not published; rejecting it ends it with nothing published.

### Taking a release back

A version that apt users have installed stays installed: a fixed version `V+1`, by the same
steps, is the remedy. To stop new installs of a bad version `V` at once:

1. `gh release edit v<V> --draft=true --repo Tsadoq/yaar`: the macOS route, the archive route
   and `latest/download` go back to the release before it.
2. Within 30 days of the last good publishing run (GitHub re-runs nothing older), open that
   run's page and re-run its job `publish` alone (Re-run this job). It reuses that run's
   artifact `release-final`, builds and signs nothing, deploys that run's site, in which
   `V`'s packages are not, and finds its own release already published with the same files,
   which it leaves as it is. After 30 days there is no step 2: `V+1` is the only way to take
   `V` out of the apt repository.

A tag that was pushed but never published is deleted with `git push --delete origin v<V>`;
nothing else exists for it.

## What was checked, and where

On 2026-11-17, on arm64, in containers inside the VM, with everything served from a local
web server, on the first packaging of the `.deb` and the install script. Nothing was fetched
from the two real addresses. The containers ran with an init as their first process, so that a
Keeper that has left is seen as gone. The steps of `.github/workflows/ci.yml` make the checks
of the rows of the test plan on every run; this table is what was seen by hand first.

| Check | Result |
| --- | --- |
| Programs built in `ubuntu:22.04`, packaged there | They ask for glibc 2.35 at most |
| `.deb` installed with apt on Ubuntu 22.04, 24.04, 26.04, Debian 12, 13 | Installs. A user who is not root starts the Keeper from `/usr/libexec/yaar-review/`; a second call finds the same one; it leaves by itself 5 s after the last call |
| The window under a headless Wayland compositor (sway), software Vulkan | Draws "YAAR" and "Keeper 0.1.0 is running" on all five, also when installed with `--no-install-recommends` |
| The window under X11 (Xwayland, Xvfb), software Vulkan | Draws on Ubuntu 24.04, 26.04 and Debian 13. On Ubuntu 22.04 and Debian 12 the window is created and stays empty |
| Repository built with a test key; `install.sh` piped from the local server | Installs from the repository on all five, as root and as a user with `sudo`; a second run changes nothing; `--uninstall` removes the package, the key, the source file and apt's copy of the index |
| The `.deb` changed by one byte after signing | apt: `Hash Sum mismatch`, exit 100. The script then exits 1 and leaves no key and no source file |
| Script cut off, archive with a wrong checksum, no `sudo`, unknown processor | Each exits with an error and changes nothing |
| Archive route on Fedora 45, as a user who is not root | Installs under `~/.local`; after `dnf install libxcb libxkbcommon libxkbcommon-x11` both programs run |
| macOS route, on the Mac, into a temporary folder, with the ad hoc signed app | Installs, replaces, removes; a wrong checksum and a cut-off script change nothing |
| `shellcheck` 0.9.0, `desktop-file-validate` | No finding |

## Not checked, and open

- A real desktop session, a hardware Vulkan driver, a launcher showing the entry and icon.
- X11 on Ubuntu 22.04 and Debian 12 with a hardware driver. With Mesa's software Vulkan
  of those releases the window stayed empty; the cause is not isolated.
- The window's app id, `io.github.tsadoq.yaar`, was set in the code after these runs. A
  throwaway build with the same line reported it under Wayland and as `WM_CLASS` under
  X11; the packages built from the committed code were not looked at again.
- GitHub Pages and a release's download address: HTTPS, redirects, caching. Only a run that
  publishes reaches them.
- `Maintainer` in the `.deb` carries the no-reply address of the repository owner's GitHub
  account. Whether a real contact address should stand there is the owner's call.
- The programs are not stripped (`lintian`: `unstripped-binary-or-object`), and the
  package has no changelog and no manual page.
- Ubuntu's container images drop everything under `/usr/share/doc` except `copyright`
  and changelogs (`path-exclude` in their dpkg configuration), so the notices are in the
  package and absent after an install there. Debian's images keep them.
