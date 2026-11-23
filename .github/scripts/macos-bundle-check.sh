#!/bin/bash
# The step "What the Mac bundle holds" of the job macos: the app as it ships, read from the
# bundle.
#
#   .github/scripts/macos-bundle-check.sh <YAAR.app> <version> <notices>
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
#
# Exit codes: 0 all hold; 1 one does not; 2 usage error.
#
# Exit 99 with "not built yet": the contract of slice 10 fixed this interface; its
# package fills the body.
set -eu
echo "macos-bundle-check.sh: not built yet" >&2
exit 99
