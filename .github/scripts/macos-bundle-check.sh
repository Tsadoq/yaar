#!/bin/bash
# The step "What the Mac bundle holds" of the jobs macos and release-sign-macos: the app as
# it ships, read from the bundle, and the two files users download, read from the files.
#
#   .github/scripts/macos-bundle-check.sh <YAAR.app> <version> <notices> [<packages-dir>]
#
# Fails with a sentence naming what is wrong when any of these does not hold:
#   - Contents/Info.plist: CFBundleIdentifier io.github.tsadoq.yaar, CFBundleExecutable
#     yaar-review, CFBundlePackageType APPL, CFBundleShortVersionString and CFBundleVersion
#     both <version>, LSMinimumSystemVersion 12.0, and neither LSUIElement nor
#     LSBackgroundOnly set (the bundle is an application with a Dock icon: PKG-08);
#   - Contents/MacOS holds yaar-review only; Contents/Helpers holds exactly yaar-keeper,
#     yaar-claude-hook, yaar-claude-tools and yaar-pi-report; each of the five is executable
#     and `lipo -archs` gives the processors YAAR_EXPECT_ARCHS names (default "x86_64 arm64";
#     a builder on a Mac with one target sets "arm64");
#   - each of the five is signed under the hardened runtime (`codesign -dv` shows
#     flags=0x10000(runtime)) with no entitlement (`codesign -d --entitlements -` prints
#     none), and with its identifier: io.github.tsadoq.yaar (the bundle),
#     io.github.tsadoq.yaar.keeper, io.github.tsadoq.yaar.claude-hook,
#     io.github.tsadoq.yaar.claude-tools, io.github.tsadoq.yaar.pi-report;
#   - no file named standin-lsp, standin-agent or yaar-script anywhere in the bundle;
#   - Contents/Resources/THIRD-PARTY-NOTICES.txt has the bytes of <notices>, and
#     Contents/Resources/LICENSE is the LICENSE of the repository.
# With <packages-dir>, which holds yaar-review-<version>-macos-universal.zip and .dmg made
# from that app, also:
#   - the .zip unpacked with `ditto -x -k` into a temporary folder holds YAAR.app and nothing
#     else, and that app is <YAAR.app> file for file and byte for byte (`diff -r`, which also
#     names a file one side lacks);
#   - the .dmg mounted read-only (`hdiutil attach -nobrowse -readonly -mountpoint <temp>`,
#     detached at the end whatever happens) holds YAAR.app and a link Applications to
#     /Applications, and that app is <YAAR.app> in the same way; so the notices and the
#     Info.plist users get by either file are those checked above.
#
# Exit codes: 0 all hold; 1 one does not; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "macos-bundle-check.sh: not built yet" >&2
exit 99
