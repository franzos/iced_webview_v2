# Iced_webview

[![Rust](https://github.com/franzos/iced_webview_v2/actions/workflows/rust.yml/badge.svg)](https://github.com/franzos/iced_webview_v2/actions/workflows/rust.yml)
[![crates.io](https://img.shields.io/crates/v/iced_webview_v2.svg)](https://crates.io/crates/iced_webview_v2)

A library to embed Web views in iced applications

This library supports
- [Blitz] — Rust-native HTML/CSS renderer (Stylo + Taffy + Vello), modern CSS (flexbox, grid), no JS
- [litehtml] — lightweight CPU-based HTML/CSS rendering, no JS or navigation (good for static content like emails)
- [Servo] — full browser engine (HTML5, CSS3, JS via SpiderMonkey), software-rendered
- [CEF] — Chromium Embedded Framework via cef-rs, full Chromium browser compat (HTML5, CSS3, JS)

## Compatibility

| iced | iced_webview |
|------|--------------|
| 0.14 | 0.0.9+       |
| 0.13 | 0.0.5        |

## Requirements

- Rust 1.90+ (Blitz crates from git use edition 2024, declared MSRV 1.90)
- litehtml requires `clang`/`libclang` for building `litehtml-sys`
- Servo requires `fontconfig`, `make`, `cmake`, `clang` (recent version), and `nasm` at build time
- CEF downloads the Chromium Embedded Framework binary (~200-300 MB) at build time; requires subprocess handling (see below)
- CEF on Guix: use `manifest-cef.scm` with FHS emulation:
  ```sh
  guix shell --container --emulate-fhs --network \
    --share=$HOME/.cargo --share=$HOME/.cache \
    --expose=$XDG_RUNTIME_DIR --expose=/var/run/dbus \
    -m manifest-cef.scm -- sh -c \
    "XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR WAYLAND_DISPLAY=$WAYLAND_DISPLAY \
     DBUS_SESSION_BUS_ADDRESS=$DBUS_SESSION_BUS_ADDRESS \
     CARGO_TARGET_DIR=target-cef LD_LIBRARY_PATH=/lib:/lib/nss CC=gcc \
     cargo run --example webview --no-default-features --features cef"
  ```

### Default engine

The default feature is `litehtml` — it's lightweight, pure-crate.io, and compiles fast. Blitz and Servo are git-only deps and can't be published to crates.io, so they require `--features blitz` or `--features servo` explicitly.

#### examples:

##### `examples/webview`
Minimal example — just the web view, nothing else
```sh
cargo run --release --example webview
# or with blitz
cargo run --example webview --no-default-features --features blitz
# or with servo
cargo run --example webview --no-default-features --features servo
# or with cef
cargo run --example webview --no-default-features --features cef
```

##### `examples/embedded_webview`
A simple example to showcase an embedded webview (uses the basic webview)
![image](https://raw.githubusercontent.com/franzos/iced_webview_v2/refs/heads/main/assets/embedded_webview.png)
```sh
cargo run --example embedded_webview
# or with litehtml
cargo run --example embedded_webview --no-default-features --features litehtml
# or with servo
cargo run --example embedded_webview --no-default-features --features servo
# or with cef
cargo run --example embedded_webview --no-default-features --features cef
```

##### `examples/multi_webview`
A more advanced example that uses the advanced webview module and has two simultaneous webviews open
![image](https://raw.githubusercontent.com/franzos/iced_webview_v2/refs/heads/main/assets/multi_view.png)
```sh
cargo run --example multi_webview
# or with litehtml
cargo run --example multi_webview --no-default-features --features litehtml
# or with servo
cargo run --example multi_webview --no-default-features --features servo
# or with cef
cargo run --example multi_webview --no-default-features --features cef
```

##### `examples/email`
Renders a table-based marketing email — works with any engine, but designed to showcase litehtml's table layout
```sh
cargo run --example email --no-default-features --features litehtml
# or with blitz
cargo run --example email
# or with servo
cargo run --example email --no-default-features --features servo
# or with cef
cargo run --example email --no-default-features --features cef
```

## Known Issues

Blitz and litehtml are not full browsers — there's no JavaScript, and rendering is CPU-based. Both are best suited for displaying static or semi-static HTML content. Servo and CEF are full browser engines with JS support but add significant binary size.

### Blitz

- **No incremental rendering** — the entire visible viewport is re-rasterized on every frame that needs updating (scroll, resize, resource load). Blitz is pre-alpha and doesn't yet support dirty-rect or partial repaint like Firefox/Chrome.
- **No `:hover` CSS rendering** — hover state is tracked internally (cursor changes work), but we skip the visual re-render for `:hover` styles to avoid the CPU cost. This matches litehtml's behaviour.
- **No keyboard input** — blitz-dom supports keyboard events internally (text input, Tab navigation, copy/paste), but iced_webview does not wire iced keyboard events through to the Blitz document yet. The handler is a no-op.
- **No JavaScript** — by design; Blitz is a CSS rendering engine, not a browser engine.
- **Image/CSS fetching is internal** — Blitz uses `blitz_net::Provider` to fetch sub-resources (images, CSS `@import`) automatically. It does not participate in the widget layer's manual image pipeline (`take_pending_images`/`load_image_from_bytes`). The widget layer fetches the initial HTML page for URL navigation, but all sub-resource loading is handled by Blitz internally.
- **Build weight** — Stylo (Firefox's CSS engine) adds significant compile time on first build.

### litehtml

- **Limited CSS support** — no flexbox, no grid, no CSS variables. Works well for table-based layouts and simple pages (emails, documentation).
- **No `:hover` CSS rendering** — cursor changes work, but hover styles are not visually applied.
- **No JavaScript or navigation history** — static rendering only.
- **C++ dependency** — requires `clang`/`libclang` for building `litehtml-sys`.

### Servo

- **Git-only dependency** — `libservo` is not on crates.io, so the `servo` feature cannot be published. Build from git only.
- **Large binary** — adds 50-150+ MB to the final binary due to SpiderMonkey and Servo's full rendering pipeline.
- **System deps** — needs `fontconfig`, `make`, `cmake`, `clang` (recent version), and `nasm` at build time.
- **No text selection** — not yet wired up through the embedding API.
- **Intermittent SpiderMonkey crashes** — servo's JS engine can segfault during script execution on certain pages (`JS::GetScriptPrivate`). This is an upstream servo/SpiderMonkey issue, not specific to the embedding. Pages with heavy JS are more likely to trigger it.
- **Rendering** — uses iced's `shader` widget with a persistent GPU texture updated in-place via `queue.write_texture()` each frame. This avoids the texture cache churn (and visible flickering) that would otherwise occur with iced's image Handle path during rapid frame updates like scrolling.

### CEF

- **Multi-process mode** — CEF runs with standard multi-process architecture (renderer, GPU, utility subprocesses). On non-FHS systems (Guix, Nix), use an FHS-emulated container (`guix shell --container --emulate-fhs`) so subprocesses can find `.pak` resources, `icudtl.dat`, and shared libraries at standard paths. Call `cef_subprocess_check()` at the top of `main()`.
- **Large runtime** — ships ~200-300 MB of Chromium binaries alongside your application.
- **Not Rust-native** — C++ under the hood, Rust bindings via [cef-rs](https://github.com/tauri-apps/cef-rs).
- **CEF binary download** — the `cef-dll-sys` build script downloads the CEF binary distribution at build time.
- **Rendering** — uses iced's `shader` widget with a persistent GPU texture updated in-place via `queue.write_texture()` each frame, same as Servo. This avoids the texture cache churn (and visible flickering) that would otherwise occur with iced's image Handle path.

## TODO

- **Blitz incremental layout** — `blitz-dom` has a feature-gated `incremental` flag that enables selective cache clearing and damage propagation in `resolve()`. Currently experimental (incomplete FC root detection, no tests), but once stabilized it would make re-layout after hover/resource loads much cheaper by only updating affected subtrees instead of the full tree.
- **`:hover` CSS rendering** — both engines skip the visual re-render for hover styles. With incremental layout + viewport-only rendering, it may become cheap enough to re-enable for Blitz.
- **Async rendering** — rendering currently blocks the main thread. Moving the `paint_scene` + `render_to_buffer` call to a background thread would keep the UI responsive during re-renders.
- **Servo text selection** — wire up Servo's text selection API through the engine trait.
- **Blitz keyboard input** — wire iced keyboard events through to `HtmlDocument::handle_ui_event` as `UiEvent::KeyDown`/`KeyUp`, enabling text input in `<input>`/`<textarea>` elements.

## Engine Comparison

| Feature | Blitz | litehtml | Servo | CEF |
|---------|-------|----------|-------|-----|
| **CSS flexbox / grid** | Yes (Firefox's Stylo engine) | No | Yes | Yes |
| **CSS variables** | Yes | No | Yes | Yes |
| **Table layout** | Yes | Yes | Yes | Yes |
| **JavaScript** | No | No | Yes (SpiderMonkey) | Yes (V8) |
| **Keyboard input** | Supported in blitz-dom, not yet wired | No | Yes | Yes |
| **Text selection** | No (not yet in blitz-dom) | Yes | No (not yet wired) | Yes (Chromium-managed) |
| **`:hover` CSS styles** | Tracked, not rendered (CPU cost) | Tracked, not rendered | Yes | Yes |
| **Cursor changes** | Yes | Yes | Yes | Yes |
| **Link navigation** | Yes | Yes | Yes | Yes |
| **Image loading** | Yes (blitz-net, automatic) | Yes (manual fetch pipeline) | Yes (built-in) | Yes (built-in) |
| **CSS `@import`** | Yes (blitz-net) | Yes (recursive fetch + cache) | Yes (built-in) | Yes (built-in) |
| **Scrolling** | Yes | Yes | Yes (engine-managed, cursor-targeted) | Yes (engine-managed) |
| **Rendering path** | iced image Handle | iced image Handle | iced shader widget (direct GPU texture) | iced shader widget (direct GPU texture) |
| **Incremental rendering** | No (experimental flag exists) | No | Yes | Yes |
| **Navigation history** | No | No | Yes | Yes |
| **Build deps** | Pure Rust | C++ (`clang`/`libclang`) | Pure Rust (git-only) | C++ (CEF binary download) |
| **Rendering performance** | Low (Stylo + Vello CPU, needs `--release`) | Moderate | Best (full rendering pipeline) | Best (full Chromium pipeline) |
| **Binary size impact** | Moderate | Small | Large (50-150+ MB) | Large (~200-300 MB runtime) |
| **License** | MIT/Apache-2.0 + MPL-2.0 (Stylo) | BSD | MPL-2.0 | MIT/Apache-2.0 (bindings) + BSD (CEF) |

[Blitz]: https://github.com/DioxusLabs/blitz
[litehtml]: https://github.com/franzos/litehtml-rs
[Servo]: https://servo.org/
[CEF]: https://github.com/tauri-apps/cef-rs

Original developer: [LegitCamper/iced_webview](https://github.com/LegitCamper/iced_webview) (Sawyer Bristol and others)
