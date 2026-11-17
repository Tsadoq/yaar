# The one patch to GPUI

`vendor/gpui-pre-apple-<version>/` is the crate `gpui-pre-apple` as published on
crates.io, with `gpui-frame-source.patch` applied. The workspace's `[patch.crates-io]`
entry makes every build use it. It is compiled on macOS only.

## What the patch changes

One function, `MetalFrameSource::is_available` in `src/display_link.rs`, and a notice at
the top of that file.

Since 2026-10-29 GPUI paces frames on macOS 14 and later with `CAMetalDisplayLink`. On
the reference Mac that puts a typed key on screen about 15 ms later than the older
`CVDisplayLink` source, which GPUI keeps for older systems. With the patch the older
source is the default. `YAAR_FRAME_SOURCE=metal` selects the newer one.

| Key to screen, behind Ghostty | Median | 95th percentile |
| --- | --- | --- |
| GPUI as published | 21.7 ms | 21.4 ms |
| With the patch, and a redraw for one second after each key | 8.3 ms | 6.1 ms |

Not tested: Macs with an M5 processor, where the older source may bring back the
30 frames a second the newer one was written to fix.

## Raising GPUI

```sh
patches/vendor-gpui.sh <new version>
cargo update -p gpui-pre-apple
```

Then run the tests and the speed check on the reference Mac. If the patch no longer
applies, GPUI changed the function: read its new form before rewriting the patch, and
remove the patch altogether if the newer frame source no longer costs anything.

The script checks the download against `gpui-pre-apple.sha256` and refuses a version
that has no line there.
