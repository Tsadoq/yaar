#!/bin/sh
# The step "Package check" of package-linux: what the .deb and the archive of one processor
# hold, read from the files, before any install.
#
#   .github/scripts/linux-package-check.sh <dist> <version> <notices>
#
# <dist> holds yaar-review_<version>_<arm64|amd64>.deb and
# yaar-review-<version>-linux-<aarch64|x86_64>.tar.gz for the processor of this machine.
# Fails with a sentence naming the file and the line when any of these does not hold:
#   - the .deb's Package is yaar-review, Version is <version>, Architecture the machine's;
#   - its Depends names, besides what dpkg-shlibdeps found: libwayland-client0, libvulkan1,
#     "mesa-vulkan-drivers | vulkan-icd", fonts-dejavu-core, git and ca-certificates;
#   - it holds exactly these programs, each executable: /usr/bin/yaar-review,
#     /usr/libexec/yaar-review/yaar-keeper, /usr/libexec/yaar-review/yaar-claude-hook,
#     /usr/libexec/yaar-review/yaar-claude-tools, /usr/libexec/yaar-review/yaar-pi-report;
#     and no file named standin-lsp, standin-agent or yaar-script anywhere;
#   - it holds the desktop entry /usr/share/applications/io.github.tsadoq.yaar.desktop, the
#     icon, and usr/share/doc/yaar-review/{copyright,NOTICE,THIRD-PARTY-NOTICES.txt.gz},
#     the last one gunzipping to the bytes of <notices>;
#   - the archive holds the same programs under yaar-review/bin and yaar-review/libexec/
#     yaar-review, the same desktop entry and icon under yaar-review/share, LICENSE, NOTICE
#     and THIRD-PARTY-NOTICES.txt (the bytes of <notices>) under yaar-review/share/doc/
#     yaar-review;
#   - in the container of the job: `apt-get install ./<the .deb>` installs it, each of the
#     five installed programs' `ldd` lists no "not found", and `yaar-review --version` prints
#     "yaar-review <version>".
#
# Exit codes: 0 all hold; 1 one does not; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "linux-package-check.sh: not built yet" >&2
exit 99
