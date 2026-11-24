# The one patch to GPUI

| Abbreviation | Meaning |
| --- | --- |
| GPUI | The interface toolkit the Window draws with, here the crate `gpui-pre-apple` |

`vendor/gpui-pre-apple-<version>/` is the crate `gpui-pre-apple` as published on
crates.io, with `gpui-frame-source.patch` applied. The `[patch.crates-io]` entry of the
root `Cargo.toml` makes every build use it. It is compiled on macOS only.

## What the patch changes

One function, `MetalFrameSource::is_available` in `src/display_link.rs`, and a notice at
the top of that file.

On macOS 14 and later, GPUI as published paces frames with `CAMetalDisplayLink`. On a
Mac, that puts a typed key on screen about 15 ms later than the older
`CVDisplayLink` source, which GPUI keeps for older systems. With the patch the older source
is the default, and `YAAR_FRAME_SOURCE=metal` selects the newer one.

| Key to screen, behind Ghostty | Median | 95th percentile |
| --- | --- | --- |
| GPUI as published | 21.7 ms | 21.4 ms |
| With the patch, and a redraw for one second after each key | 8.3 ms | 6.1 ms |

Macs with an M5 processor are not tested. There the older source may bring back the 30
frames a second the newer one fixes.

## Raising GPUI

```sh
patches/vendor-gpui.sh <new version>
cargo update -p gpui-pre-apple
```

The script downloads the published crate, checks it against `gpui-pre-apple.sha256`,
which must hold a line for that version, unpacks it into `vendor/`, applies the patch and
points the `[patch.crates-io]` entry at the new folder.
Then run the tests and the typing check on a Mac. If the patch no longer
applies, GPUI changed the function: read its new form before rewriting the patch, and
remove the patch if the newer frame source no longer costs anything.
