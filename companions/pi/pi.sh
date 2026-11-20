#!/bin/sh
# The wrapper of the app YAAR for pi. The pi Adapter writes it into its own folder of the
# app as `bin/pi`, with @REPORT@ and @EXTENSION@ replaced by the absolute paths of the
# reporting program and of the extension, each single-quoted, and puts that folder first on
# the search path of every shell of the app. Typed as `pi`, it runs the next `pi` on the
# search path after its own folder with `-e` and the extension, and every argument it was
# given, as they are; when the first argument is a subcommand of pi's, with nothing added.
# It exports YAAR_PI_REPORT, which the extension finds the reporting program by. It writes
# nothing. With no other `pi` on the search path it says so and ends with 127.
#
# The list of subcommands is the crate constant `yaar_adapter_pi::SUBCOMMANDS`; a test of
# the crate holds the two equal.

YAAR_PI_REPORT=@REPORT@
export YAAR_PI_REPORT
extension=@EXTENSION@

echo "the app's pi wrapper is not built yet" >&2
exit 70
