The release's pull request copies this file, from the heading below, into `README.md` under "Build", with `@BASE_URL@` replaced by the line of `packaging/site/base-url`.

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
