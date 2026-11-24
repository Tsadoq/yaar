# Releasing YAAR

| Abbreviation | Meaning |
| --- | --- |
| API | The App Store Connect interface that `notarytool` uses |
| UUID | A universally unique identifier |

## What a release holds

- Six packages: the `.deb` and the plain archive for each Linux processor, and the Mac
  app, universal, as a `.zip` and a `.dmg`.
- `SHA256SUMS`, one line for each package, and `SHA256SUMS.asc`, its signature by the
  project's key.
- On the site, which GitHub Pages serves: `index.html`, `install.sh` and the apt
  repository under `apt/`.

A release is the workflow `ci` run by hand on a version tag with the input `publish` on.
Every other run is a rehearsal and publishes nothing. A run that publishes signs the Mac
app with the Developer ID, has Apple notarise it, signs the apt repository and
`SHA256SUMS` with the project's key, and waits for the owner's approval. Once approved, it
makes a draft release with the eight files, deploys the site, and publishes the release as
the latest. A published version is never changed.

The base address of the site is the one line of `packaging/site/base-url`. The site's
`install.sh` and `index.html` and the release notes take it from there. A text that shows
it holds a copy, which `.github/scripts/base-url-check.sh` holds to that line.

## The secrets, and two public files

Six secrets of the environment `release`, which only the jobs `release-sign-macos` and
`assemble-release` name, and which the environment gives to runs on the tags `v*` only.

| Secret | What it holds | Form |
| --- | --- | --- |
| `MACOS_CERTIFICATE_P12` | The Developer ID Application certificate with its private key, and Apple's Developer ID Certification Authority (G2) certificate | The `.p12` file the two were exported into together, in base64 |
| `MACOS_CERTIFICATE_PASSWORD` | The password the `.p12` was exported with | Text |
| `APPLE_NOTARY_KEY_P8` | An App Store Connect API key, for `notarytool` | The text of the file `AuthKey_<key id>.p8` |
| `APPLE_NOTARY_KEY_ID` | That key's id | Text, ten characters |
| `APPLE_NOTARY_ISSUER_ID` | The issuer id of the team's keys | Text, a UUID |
| `APT_SIGNING_KEY` | The project's OpenPGP key that signs the apt repository and `SHA256SUMS` | The armoured secret key, ed25519, no passphrase on this copy |

Two public files hold one line each, under their comments:
`packaging/apt/project-key.fingerprint`, the apt key's fingerprint, and
`packaging/macos/team-id`, the Team ID of the Developer ID. The workflow refuses an apt key
of another fingerprint and a certificate of another team, and the install script refuses
an app that another team signed. The apt key is made once and never replaced: a client
that trusts the old key stops receiving updates.

## Before the first release, once

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

4. The notary key. The Account Holder of the developer account requests access to the App
   Store Connect API once (Users and Access > Integrations > App Store Connect API,
   "Request Access") and accepts its terms. Then, on the same page, under Team Keys, add a
   key with the access Developer and download it (it can be downloaded once). The first
   publishing run checks the key with Apple before it signs anything. The page shows the
   Key ID and the Issuer ID. Then:

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

## The day of a release

1. Raise the version in `Cargo.toml` (`[workspace.package]`) in a pull request, and merge
   it. For the first release, the same pull request copies the section
   [Install](#install) below into `README.md` as its section "Install", after "What works
   today", with every `@BASE_URL@` replaced by the line of `packaging/site/base-url`, and
   takes out the line that says no release is published yet.

2. From the merged commit, tag and start the publishing run:

   ```sh
   git tag v<version> && git push origin v<version>        # a rehearsal runs on the tag
   gh workflow run ci.yml --repo Tsadoq/yaar --ref v<version> -f publish=true
   ```

   The run refuses a tag that is not `v` and the version of `Cargo.toml`, and a tag whose
   commit is not on `main`. When every job before `publish` is green, the run waits for
   the approval.

3. Meanwhile, walk the checklists on the files the run made. Download them and serve them
   on this machine:

   ```sh
   gh run download <run id> --repo Tsadoq/yaar -n release-final -D ~/yaar-release-<version>
   packaging/checklists/serve-release.sh ~/yaar-release-<version>
   ```

   It needs `python3` and `curl`. It checks the files against `SHA256SUMS`, serves them on 127.0.0.1
   only, and prints the install and the uninstall lines; run those in another terminal. On a
   Linux machine, run it there, on a copy of the same folder. Then each item, with the
   scripts of `packaging/checklists/`:

   | Item | Command |
   | --- | --- |
   | The platform's conventions, macOS | `conventions.sh macos` |
   | The platform's conventions, Linux | `conventions.sh linux` |
   | Cold start to the diff, 0.3 s | `start-time.sh` |
   | The same in the Linux virtual machine | `start-time.sh --linux` |
   | Memory, 200 MB | `memory.sh` |
   | Typing, at most 10 ms behind Ghostty | `typing.sh` |
   | Scrolling a diff of 2,200 lines | `scroll.sh` |
   | A flood of 200,000 lines | `flood.sh` |
   | The test suite, which needs `YAAR_LANGUAGE_SERVERS`, and the looks of the screen | `screen-walk.sh` |
   | The real Claude Code and pi | `real-clis.sh` |
   | Linux on real hardware | `linux-hardware.sh --server http://127.0.0.1:<port>` |

   Every script prints its usage with `--help` and writes its results into
   `~/yaar-checklists/<YYYY-MM-DD>/`. The scripts that walk a list read their answers from
   the terminal or from a file (`conventions.sh macos < answers.txt`; the format is at the
   top of `lib.sh`). `typing.sh`, `scroll.sh` and `flood.sh` ask for the word `go` before
   they post any input; `typing.sh` needs Accessibility and Screen Recording.

   `typing.sh`, `scroll.sh`, `flood.sh` and `start-time.sh --linux` need the owner's
   measuring tools and a Linux virtual machine of his, which are not in the repository.
   The three tools are found under `tools/` of the folder `YAAR_KIT` names, or one by one
   through `YAAR_FAKEINPUT`, `YAAR_ADECAP` and `YAAR_STREAM`; the program that drives the
   virtual machine is named in `YAAR_VM`. A script that lacks one says which variable to
   set.

   To check the scripts themselves without opening a window or posting input:

   ```sh
   packaging/checklists/self-test.sh
   ```

   It runs the walkers on files of answers and with `--dry-run`, `serve-release.sh` on a
   made-up folder, and the analysis of made-up logs. It needs `curl`, `lsof`, `awk`, `cmp`
   and `python3`.

4. Approve on the run's page (Review deployments), or:

   ```sh
   gh api -X POST repos/Tsadoq/yaar/actions/runs/<run id>/pending_deployments \
     -F "environment_ids[]=$(gh api repos/Tsadoq/yaar/environments/github-pages --jq .id)" \
     -f state=approved -f comment='checklists walked'
   ```

A run left waiting is not published; rejecting it ends it with nothing published.

## Taking a release back

A version that apt users have installed stays installed: a fixed version `V+1`, by the same
steps, is the remedy. To stop new installs of a bad version `V` at once:

1. `gh release edit v<V> --draft=true --repo Tsadoq/yaar`: the macOS route, the archive route
   and `latest/download` go back to the release before it.
2. Within 30 days of the last good publishing run (GitHub re-runs nothing older), open that
   run's page and re-run its job `publish` alone (Re-run this job). It reuses that run's
   artifact `release-final`, builds and signs nothing, deploys that run's site, in which
   `V`'s packages are not, and leaves its own release as it is. After 30 days there is no
   step 2: `V+1` is the only way to take `V` out of the apt repository.

A tag that was pushed but never published is deleted with `git push --delete origin v<V>`;
nothing else exists for it.

## The README's install section

The text below is the section "Install" of `README.md` once a release is published, with
`@BASE_URL@` in place of the base address.

### Install

YAAR runs on macOS 26 and 27 on Apple silicon, and on Debian 12 and 13 and Ubuntu 22.04, 24.04
and 26.04, each on x86-64 and on arm64. An Intel Mac and other Linux systems get builds that
nobody has tested.

```sh
curl -fsSL @BASE_URL@/install.sh | bash
```

On macOS the script installs the notarised app into `/Applications`, or into `~/Applications`
when that is not writable. On Debian and Ubuntu it installs the package from the signed apt
repository at `@BASE_URL@/apt`, which stays among apt's sources. On another Linux it unpacks
the plain archive under `~/.local`. Add `--dry-run` to see what it would do and change
nothing.

To remove YAAR, quit it, then:

```sh
curl -fsSL @BASE_URL@/install.sh | bash -s -- --uninstall
```

That removes the app and the files it keeps for itself in your cache, runtime and data
folders. It keeps your reviews and the extension the app wrote for pi. `--purge` removes those
too:

```sh
curl -fsSL @BASE_URL@/install.sh | bash -s -- --uninstall --purge
```

If YAAR is still running, the uninstall stops and says so, because agents may be running in
its terminals. `--stop-agents` ends YAAR and those agents:

```sh
curl -fsSL @BASE_URL@/install.sh | bash -s -- --uninstall --stop-agents
```

By hand: every release is on the [releases page](https://github.com/Tsadoq/yaar/releases) with
the `.dmg` for macOS, the `.deb` for each processor and a plain archive for each Linux
processor. `SHA256SUMS` lists them, and `SHA256SUMS.asc` is its signature by the key at
`@BASE_URL@/apt/yaar-review.asc`. To use the apt repository without the script, put the key in
place:

```sh
sudo install -d -m 0755 /etc/apt/keyrings
sudo curl -fsSL -o /etc/apt/keyrings/yaar-review.asc @BASE_URL@/apt/yaar-review.asc
```

Write `/etc/apt/sources.list.d/yaar-review.sources`:

```
Types: deb
URIs: @BASE_URL@/apt
Suites: stable
Components: main
Signed-By: /etc/apt/keyrings/yaar-review.asc
```

Then `sudo apt-get update` and `sudo apt-get install yaar-review`. To check the key, compare
its fingerprint with the one the site's page shows:

```sh
curl -fsSL @BASE_URL@/apt/yaar-review.asc | gpg --show-keys --fingerprint
```
