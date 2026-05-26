# Iced_webview

[![Rust](https://github.com/franzos/iced_webview_v2/actions/workflows/rust.yml/badge.svg)](https://github.com/franzos/iced_webview_v2/actions/workflows/rust.yml)
[![crates.io](https://img.shields.io/crates/v/iced_webview_v2.svg)](https://crates.io/crates/iced_webview_v2)

A library to embed Web views in iced applications

> **Heads up — the [`next`](https://github.com/franzos/iced_webview_v2/tree/next) branch tracks iced `master` and blitz `0.3.0-alpha.4` (both on wgpu 29).** Blitz there rasterizes directly on iced's `wgpu::Device` via a shared Vello renderer — no CPU pixel readback. Servo is temporarily disabled on that branch (stylo 0.15 vs 0.17 conflict).

This library supports
- [Blitz] — Rust-native HTML/CSS renderer (Stylo + Taffy + Vello), GPU-rasterized via wgpu, modern CSS (flexbox, grid), no JS
- [litehtml] — lightweight CPU-based HTML/CSS rendering, no JS or navigation (good for static content like emails)
- [Servo] — full browser engine (HTML5, CSS3, JS via SpiderMonkey), rendered to CPU buffer, displayed via iced shader widget
- [CEF] — Chromium Embedded Framework via cef-rs, full Chromium browser compat (HTML5, CSS3, JS)

| Blitz | litehtml | Servo | CEF |
|-------|----------|-------|-----|
| ![Blitz](assets/webview_blitz.png) | ![litehtml](assets/webview_litehtml.png) | ![Servo](assets/webview_servo.png) | ![CEF](assets/webview_cef.png) |

## Compatibility

| iced | iced_webview |
|------|--------------|
| 0.14 | 0.0.9+       |
| 0.13 | 0.0.5        |

## Usage

Add to your `Cargo.toml` (the library pulls in iced internally, but your app will need it too):

```toml
[dependencies]
iced_webview_v2 = "0.1"
iced = { version = "0.14", features = ["advanced", "image", "tokio", "lazy"] }
```

The default engine is `litehtml`. To use a different one, disable defaults and pick one:

```toml
iced_webview_v2 = { version = "0.1", default-features = false, features = ["blitz"] }  # or "servo", "cef"
```

### Minimal example

```rust
use iced::{time, Element, Subscription, Task};
use iced_webview::{Action, PageType, WebView};
use std::time::Duration;

type Engine = iced_webview::Litehtml; // or Blitz, Servo, Cef

#[derive(Debug, Clone)]
enum Message {
    WebView(Action),
    ViewCreated,
}

struct App {
    webview: WebView<Engine, Message>,
    ready: bool,
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let webview = WebView::new()
            .on_create_view(Message::ViewCreated)
            .on_action(Message::WebView);
        (
            Self {
                webview,
                ready: false,
            },
            Task::done(Message::WebView(Action::CreateView(PageType::Url(
                "https://example.com".to_string(),
            )))),
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WebView(action) => self.webview.update(action),
            Message::ViewCreated => {
                self.ready = true;
                self.webview.update(Action::ChangeView(0))
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        if self.ready {
            self.webview.view().map(Message::WebView)
        } else {
            iced::widget::text("Loading...").into()
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        // Drives rendering, image fetching, and engine state
        time::every(Duration::from_millis(10))
            .map(|_| Action::Update)
            .map(Message::WebView)
    }
}

fn main() -> iced::Result {
    // CEF requires this at the top of main()
    #[cfg(feature = "cef")]
    if iced_webview::cef_subprocess_check() {
        return Ok(());
    }

    iced::application(App::new, App::update, App::view)
        .title("Webview")
        .subscription(App::subscription)
        .run()
}
```

The periodic `Action::Update` subscription is required — it drives rendering, image fetching, and engine state. Use `PageType::Url` to load a URL, or `PageType::Html` to render a raw HTML string. Track navigation with `on_url_change` / `on_title_change`.

### Basic vs Advanced WebView

**Basic** (`iced_webview::WebView`) manages views with simple `u32` indexing — create with `Action::CreateView`, switch with `Action::ChangeView(index)`, render with `webview.view()`. Good for most use cases.

**Advanced** (`iced_webview::advanced::WebView`) gives you explicit `ViewId` control. Every action and callback includes the `ViewId`, and you render a specific view with `webview.view(id)`. Use this for multi-view scenarios like a tabbed browser.

### Rendering paths

Handled transparently — `webview.view()` returns the right widget type based on the engine feature — but worth knowing about:

- **Image Handle** (litehtml) — the engine rasterizes to a CPU pixel buffer, displayed via iced's `image::Handle`. Simple, works everywhere.
- **Shader widget** (Blitz, Servo, CEF) — uses iced's `shader` widget with a persistent GPU texture updated in-place via `queue.write_texture()`. Avoids texture cache churn and flickering during rapid updates like scrolling.

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
```sh
cargo run --example embedded_webview
# or with litehtml
cargo run --example embedded_webview --no-default-features --features litehtml
# or with servo
cargo run --example embedded_webview --no-default-features --features servo
# or with cef
cargo run --example embedded_webview --no-default-features --features cef
```

##### `examples/email`
Renders a table-based marketing email — works with any engine, but designed to showcase litehtml's table layout
```sh
cargo run --example email --no-default-features --features litehtml
# or with blitz
cargo run --example email --no-default-features --features blitz
# or with servo
cargo run --example email --no-default-features --features servo
# or with cef
cargo run --example email --no-default-features --features cef
```

## Known Issues

Blitz and litehtml are not full browsers — there's no JavaScript, and rendering is CPU-based. Both are best suited for displaying static or semi-static HTML content. Servo and CEF are full browser engines with JS support but add significant binary size.

### Blitz

- **No incremental rendering** — the entire visible viewport is re-rasterized on every frame that needs updating. Blitz is pre-alpha and doesn't yet support dirty-rect or partial repaint like Firefox/Chrome. Blitz manages its own scrolling internally (`viewport_scroll`), so the rasterized texture is bounded by window size regardless of document length.
- **GPU rasterization with CPU readback** — the viewport is rasterized on the GPU via `anyrender_vello`, then read back to a CPU buffer and re-uploaded to iced's wgpu device for display. The redundant roundtrip is unavoidable until iced and `anyrender_vello` agree on a wgpu major version (currently 27 vs 28). See [TODO.md](TODO.md).
- **`:hover` CSS rendering** — hover state changes trigger a Stylo re-resolve before paint, so `:hover` styles are visually applied (unlike litehtml).
- **Keyboard input** — iced keyboard events are wired through to blitz-dom (text input, Tab navigation, arrow keys, copy/paste). Dark mode is detected from `ICED_WEBVIEW_COLOR_SCHEME` env var or GTK theme.
- **No JavaScript** — by design; Blitz is a CSS rendering engine, not a browser engine.
- **Image/CSS fetching is internal** — Blitz uses `blitz_net::Provider` to fetch sub-resources (images, CSS `@import`) automatically. It does not participate in the widget layer's manual image pipeline (`take_pending_images`/`load_image_from_bytes`). The widget layer fetches the initial HTML page for URL navigation, but all sub-resource loading is handled by Blitz internally.
- **Build weight** — Stylo (Firefox's CSS engine) adds significant compile time on first build.

### litehtml

- **Limited CSS support** — basic flexbox, no grid, no CSS variables. Works well for table-based layouts and simple pages (emails, documentation).
- **No `:hover` CSS rendering** — cursor changes work, but hover styles are not visually applied.
- **No JavaScript or navigation history** — static rendering only.
- **C++ dependency** — requires `clang`/`libclang` for building `litehtml-sys`.

### Servo

- **Git-only dependency** — `libservo` is not on crates.io, so the `servo` feature cannot be published. Build from git only.
- **Large binary** — adds 50-150+ MB to the final binary due to SpiderMonkey and Servo's full rendering pipeline.
- **System deps** — needs `fontconfig`, `make`, `cmake`, `clang` (recent version), and `nasm` at build time.
- **Text selection** — Servo manages text selection and clipboard (Ctrl+C/V) internally, but the selection is not queryable from the embedding API (`get_selected_text()` returns None).
- **Intermittent SpiderMonkey crashes** — servo's JS engine can segfault during script execution on certain pages (`JS::GetScriptPrivate`). This is an upstream servo/SpiderMonkey issue, not specific to the embedding. Pages with heavy JS are more likely to trigger it.
- **Rendering** — Servo software-renders to a CPU buffer, which is then uploaded to a persistent GPU texture via `queue.write_texture()` and displayed through iced's `shader` widget. The texture is only updated when Servo signals a new frame. This avoids the texture cache churn (and visible flickering) that would otherwise occur with iced's image Handle path during rapid frame updates like scrolling.

### CEF

- **Multi-process mode** — CEF runs with standard multi-process architecture (renderer, GPU, utility subprocesses). On non-FHS systems (Guix, Nix), use an FHS-emulated container (`guix shell --container --emulate-fhs`) so subprocesses can find `.pak` resources, `icudtl.dat`, and shared libraries at standard paths. Call `cef_subprocess_check()` at the top of `main()`.
- **Large runtime** — ships ~200-300 MB of Chromium binaries alongside your application.
- **Not Rust-native** — C++ under the hood, Rust bindings via [cef-rs](https://github.com/tauri-apps/cef-rs).
- **CEF binary download** — the `cef-dll-sys` build script downloads the CEF binary distribution at build time.
- **Rendering** — same as Servo: CPU buffer uploaded to a persistent GPU texture via `queue.write_texture()`, displayed through iced's `shader` widget. Only updated when CEF delivers a new frame via its `on_paint` callback.

## TODO

- **Blitz zero-copy GPU rendering** — eliminate the GPU→CPU→GPU readback by sharing iced's wgpu device with `anyrender_vello`. Blocked until the two agree on a wgpu major version. See [TODO.md](TODO.md).
- **Blitz incremental layout** — `blitz-dom` has a feature-gated `incremental` flag that enables selective cache clearing and damage propagation in `resolve()`. Currently experimental (incomplete FC root detection, no tests), but once stabilized it would make re-layout after hover/resource loads much cheaper by only updating affected subtrees instead of the full tree.
- **litehtml `:hover` CSS rendering** — litehtml tracks hover state but doesn't re-render styles. Blitz now does this; same approach (resolve before paint) could be applied.
- **Async rendering** — rendering currently blocks the main thread. Moving the `paint_scene` + readback call to a background thread would keep the UI responsive during re-renders.
- **Servo/CEF text selection API** — expose the engine-managed selected text through `get_selected_text()` so the embedding can query it.

## Engine Comparison

| Feature | Blitz | litehtml | Servo | CEF |
|---------|-------|----------|-------|-----|
| **CSS flexbox / grid** | Yes (Firefox's Stylo engine) | Flexbox only (no grid) | Yes | Yes |
| **CSS variables** | Yes | No | Yes | Yes |
| **Table layout** | Yes | Yes | Yes | Yes |
| **JavaScript** | No | No | Yes (SpiderMonkey) | Yes (V8) |
| **Keyboard input** | Yes (wired to blitz-dom) | No | Yes | Yes |
| **Text selection** | Yes (drag-to-select via blitz-dom) | Yes | Yes (engine-managed, not queryable from API) | Yes (Chromium-managed, not queryable from API) |
| **`:hover` CSS styles** | Yes | Tracked, not rendered | Yes | Yes |
| **Cursor changes** | Yes | Yes | Yes | Yes |
| **Link navigation** | Yes | Yes | Yes | Yes |
| **Image loading** | Yes (blitz-net, automatic) | Yes (manual fetch pipeline) | Yes (built-in) | Yes (built-in) |
| **CSS `@import`** | Yes (blitz-net) | Yes (recursive fetch + cache) | Yes (built-in) | Yes (built-in) |
| **Scrolling** | Yes | Yes | Yes (engine-managed, cursor-targeted) | Yes (engine-managed) |
| **Rendering path** | iced shader widget (direct GPU texture) | iced image Handle | iced shader widget (direct GPU texture) | iced shader widget (direct GPU texture) |
| **Incremental rendering** | No (experimental flag exists) | No | Yes | Yes |
| **Navigation history** | No | No | Yes | Yes |
| **Build deps** | Pure Rust | C++ (`clang`/`libclang`) | Rust + system deps (git-only) | C++ (CEF binary download) |
| **Rendering performance** | Moderate (Vello GPU, viewport-only repaint, GPU→CPU readback tax) | Moderate | Best (full rendering pipeline) | Best (full Chromium pipeline) |
| **Binary size impact** | Moderate | Small | Large (50-150+ MB) | Large (~200-300 MB runtime) |
| **License** | MIT/Apache-2.0 + MPL-2.0 (Stylo) | MIT + BSD-3-Clause | MPL-2.0 | Apache-2.0 (this crate) + BSD (CEF) |

[Blitz]: https://github.com/DioxusLabs/blitz
[litehtml]: https://github.com/franzos/litehtml-rs
[Servo]: https://servo.org/
[CEF]: https://github.com/tauri-apps/cef-rs

Original developer: [LegitCamper/iced_webview](https://github.com/LegitCamper/iced_webview) (Sawyer Bristol and others)
