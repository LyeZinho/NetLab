# Asset attribution

**Policy:** NetworksCraft contains no vendor artwork, icons, fonts, documentation text, templates or
saved labs from Cisco Packet Tracer, GNS3, EVE-NG or any other product. Every visual in the app is
original work drawn with egui shapes in `src/ui.rs`; colours come from `src/theme.rs`.

Generated-in-code assets (no file to list):

- the NetworksCraft mark, device icons, tool icons and toolbar icons (`src/ui.rs`);
- the topology grid, selection rings, packet pulse and all panel chrome (`src/ui.rs`, `src/theme.rs`);
- the built-in sample and campus labs (`src/model.rs`);
- the screenshots produced by `--screenshot` (original renders of this app).

## Third-party visual assets

The following fonts are embedded by egui's `epaint_default_fonts` crate at build time. They are not
copied into this repository; they are compiled into the binary through the `egui` / `eframe`
dependency. They are listed here because they are visual assets.

| Asset | Author | Source | Licence | Notes |
|---|---|---|---|---|
| Ubuntu-Light | Canonical Ltd. and contributors | `epaint_default_fonts` 0.36.2, fonts/Ubuntu-Light.ttf | Ubuntu Font Licence 1.0 | egui default proportional font |
| Hack-Regular | Source Foundry Authors; derived in part from DejaVu (public domain) and Bitstream Vera | `epaint_default_fonts` 0.36.2, fonts/Hack-Regular.ttf | MIT / Bitstream Vera Licence | egui default monospace font |
| NotoEmoji-Regular | Google Inc. | `epaint_default_fonts` 0.36.2, fonts/NotoEmoji-Regular.ttf | SIL Open Font License 1.1 | egui emoji fallback |
| emoji-icon-font | John Slegers | `epaint_default_fonts` 0.36.2, fonts/emoji-icon-font.ttf | MIT | egui icon fallback |

## Software dependencies (not assets)

| Crate | Licence | Use |
|---|---|---|
| `eframe`, `egui` | MIT OR Apache-2.0 | desktop window, immediate-mode UI |
| `serde`, `serde_json` | MIT OR Apache-2.0 | model-independent lab wire format |
| `thiserror` | MIT OR Apache-2.0 | library error types |
| `image` | MIT OR Apache-2.0 | PNG screenshot export |

The repository's code is MIT OR Apache-2.0; see the root `LICENSE-MIT`, `LICENSE-APACHE` and
`NOTICE` files.
