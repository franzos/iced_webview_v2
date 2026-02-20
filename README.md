# Iced_webview

[![Rust](https://github.com/franzos/iced_webview_v2/actions/workflows/rust.yml/badge.svg)](https://github.com/franzos/iced_webview_v2/actions/workflows/rust.yml)
[![crates.io](https://img.shields.io/crates/v/iced_webview_v2.svg)](https://crates.io/crates/iced_webview_v2)

A library to embed Web views in iced applications

This library supports
- [Blitz] — Rust-native HTML/CSS renderer (Stylo + Taffy + Vello), modern CSS (flexbox, grid), no JS
- [litehtml] — lightweight CPU-based HTML/CSS rendering, no JS or navigation (good for static content like emails)

## Compatibility

| iced | iced_webview |
|------|--------------|
| 0.14 | 0.0.9+       |
| 0.13 | 0.0.5        |

## Requirements

- Rust 1.85+ (Blitz crates use edition 2024)

#### examples:

##### `examples/embedded_webview`
A simple example to showcase an embedded webview (uses the basic webview)
![image](https://raw.githubusercontent.com/franzos/iced_webview_v2/refs/heads/main/assets/embedded_webview.png)
```sh
cargo run --example embedded_webview
# or with litehtml
cargo run --example embedded_webview --no-default-features --features litehtml
```

##### `examples/multi_webview`
A more advanced example that uses the advanced webview module and has two simultaneous webviews open
![image](https://raw.githubusercontent.com/franzos/iced_webview_v2/refs/heads/main/assets/multi_view.png)
```sh
cargo run --example multi_webview
```

##### `examples/email`
Renders a table-based marketing email using litehtml — demonstrates static HTML rendering without a full browser engine
```sh
cargo run --example email --no-default-features --features litehtml
```

## Known Issues

Neither engine is a full browser — there's no JavaScript, and rendering is CPU-based. Both are best suited for displaying static or semi-static HTML content.

### Blitz

- **No incremental rendering** — the entire visible viewport is re-rasterized on every frame that needs updating (scroll, resize, resource load). Blitz is pre-alpha and doesn't yet support dirty-rect or partial repaint like Firefox/Chrome.
- **No `:hover` CSS rendering** — hover state is tracked internally (cursor changes work), but we skip the visual re-render for `:hover` styles to avoid the CPU cost. This matches litehtml's behaviour.
- **No JavaScript** — by design; Blitz is a CSS rendering engine, not a browser engine.
- **Build weight** — Stylo (Firefox's CSS engine) adds significant compile time on first build.

### litehtml

- **Limited CSS support** — no flexbox, no grid, no CSS variables. Works well for table-based layouts and simple pages (emails, documentation).
- **No `:hover` CSS rendering** — cursor changes work, but hover styles are not visually applied.
- **No JavaScript or navigation history** — static rendering only.
- **C++ dependency** — requires `clang`/`libclang` for building `litehtml-sys`.

## TODO

- **Blitz incremental layout** — `blitz-dom` has a feature-gated `incremental` flag that enables selective cache clearing and damage propagation in `resolve()`. Currently experimental (incomplete FC root detection, no tests), but once stabilized it would make re-layout after hover/resource loads much cheaper by only updating affected subtrees instead of the full tree.
- **`:hover` CSS rendering** — both engines skip the visual re-render for hover styles. With incremental layout + viewport-only rendering, it may become cheap enough to re-enable for Blitz.
- **Async rendering** — rendering currently blocks the main thread. Moving the `paint_scene` + `render_to_buffer` call to a background thread would keep the UI responsive during re-renders.
- **Servo integration** — a full Rust-native browser engine (HTML5, CSS3, JS) as a third engine option. Servo's embedding API (`libservo`) is stabilizing but not yet on crates.io.

## Engine Comparison

| Feature | Blitz | litehtml |
|---------|-------|----------|
| **CSS flexbox / grid** | Yes (Firefox's Stylo engine) | No |
| **CSS variables** | Yes | No |
| **Table layout** | Yes | Yes |
| **JavaScript** | No | No |
| **Text selection** | No (not yet in blitz-dom) | Yes |
| **`:hover` CSS styles** | Tracked, not rendered (CPU cost) | Tracked, not rendered |
| **Cursor changes** | Yes | Yes |
| **Link navigation** | Yes | Yes |
| **Image loading** | Yes (blitz-net, automatic) | Yes (manual fetch pipeline) |
| **CSS `@import`** | Yes (blitz-net) | Yes (recursive fetch + cache) |
| **Scrolling** | Yes | Yes |
| **Incremental rendering** | No (experimental flag exists) | No |
| **Build deps** | Pure Rust | C++ (`clang`/`libclang`) |
| **License** | MIT/Apache-2.0 + MPL-2.0 (Stylo) | BSD |

[Blitz]: https://github.com/DioxusLabs/blitz
[litehtml]: https://github.com/franzos/litehtml-rs

Original developer: [LegitCamper/iced_webview](https://github.com/LegitCamper/iced_webview) (Sawyer Bristol and others)
