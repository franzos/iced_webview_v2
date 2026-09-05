# Changelog

## [Unreleased] — `iced_main_blitz_main` branch

### Security
- Fixed a panic on malformed CSS (`@import` at end of stylesheet) triggerable by remote content
- Download size limits are enforced while streaming instead of after buffering the whole body
- Capped images fetched per page (128) and clamped litehtml render height to prevent memory exhaustion from hostile pages
- Updated dependencies with known advisories (rustls-webpki, quinn-proto, tar, thin-vec)
- Documented the disabled CEF sandbox and the fetch pipeline's SSRF exposure

### Fixed
- CEF: initialization is guarded once-per-process; a second engine instance fails cleanly instead of corrupting CEF state
- Servo: the rendering context is resized with the view; empty frames are logged instead of silently kept stale
- Page titles are reported for the Blitz and litehtml engines (`on_title_change`)
- Wrong gamma for CPU-rendered engines on sRGB surfaces (builds without the `blitz` feature)
- Stale image fetches from a previous page no longer corrupt the current page's fetch tracking
- litehtml: container access reworked to raw-pointer provenance, removing an aliasing soundness hole

### Changed
- Blitz rasterizes directly on iced's `wgpu::Device` via a shared Vello renderer
- iced pinned to `master`, blitz to `0.3.0-beta.2`, Servo to `0.5` (stylo 0.20, wgpu 29)
- Widget impls migrated to iced master's `Widget` trait (no `Clipboard` param on `update`)
- Shader texture format switched to linear `Rgba8Unorm`
- GPU texture upload is skipped when the frame is unchanged; removed a per-frame full-buffer clone on the shader path
- Errors and warnings go through the `log` crate instead of stderr
- View IDs use a monotonic counter; the `rand` dependency was removed
- URL/title polling happens on update ticks and navigation only, not on every input event
- Stylesheets are fetched concurrently (limit 8) instead of sequentially
- `Engine` trait: removed unused `focus`/`unfocus` methods and ignored size parameters
- Added `Servo::try_new`, `Cef::try_new`, and `WebView::with_engine` for fallible engine construction
- Widget logic duplicated between `basic` and `advanced` moved to an internal shared module

### Added
- `engines::GpuFrame` / `GpuFrameHandle` types and `Engine::gpu_frame()` trait method

### Removed
- `GpuRasterizer` / `VelloImageRenderer` CPU-buffer path in the blitz engine

## [0.1.11] - 2026-05-26

### Fixed
- Automatic HiDPI scale-factor detection — no manual `set_scale_factor` needed

## [0.1.10] - 2026-05-26

### Fixed
- HiDPI text sharpness: webview content now blits 1:1 with the surface (nearest-neighbor sampling + rounded physical sizes) instead of being bilinear-resampled
- Servo HiDPI: content was rendered at 2× size and upscaled (double-applied the display scale); Servo now paints a physical-resolution buffer matching its hidpi factor
- Stale/inaccurate code comments around engine render-path routing and texture color space

## [0.1.9] - 2026-05-25

### Fixed
- litehtml email rendering: nested table content no longer inherits `text-align: center` from a `<td align="center">` wrapper (via litehtml 0.2.5)
- HiDPI rendering: litehtml/blitz content was stretched vertically by the display scale factor

## [0.1.8] - 2026-05-05

### Changed
- Blitz engine switched from CPU rasterization (`anyrender_vello_cpu`) to GPU rasterization (`anyrender_vello`)
- Blitz rendering routed through iced's shader widget instead of `image::Handle` — avoids viewport-sized Handle clones per frame
- Blitz scroll handling delegated to the engine via `viewport_scroll` / `Wheel` events — drops the manual `scroll_y` / `content_height` bookkeeping
- Persistent `GpuRasterizer` shared across views so wgpu/Vello pipeline init happens once, not per frame
- `ImageInfo::from_shader_pixels` skips the `image::Handle` allocation on the shader path

### Added
- Blitz `:hover` CSS rendering — resolve runs before paint so hover styles appear visually
- Blitz drag-selection — `PointerMove` carries live button mask so Blitz can drive text selection
- Blitz event-driven redraw — `ShellProvider::request_redraw` signals from scroll, hover, IME, resource arrival
- `TODO.md` documenting Stylo 0.16 and wgpu version blockers

## [0.1.7] - 2026-04-21

### Added
- Servo engine: event-driven wake subscription via `webview.subscription()` — replaces hardcoded `time::every(...)` polling
- `ServoWaker` signals the embedder only when Servo has work, with a 500ms fallback tick as a safety net
- Examples updated to use the new Servo subscription when the `servo` feature is enabled

### Changed
- `servo` feature now depends on `tokio`; `tokio` dependency gains the `sync` feature for `Notify`

## [0.1.6] - 2026-04-03

### Added
- `.with_initial_size()` builder method for setting viewport size before first resize
- `current_url()` / `current_title()` getters on basic WebView
- `url_for(id)` / `title_for(id)` getters on advanced WebView
- Doc comments on WebView structs noting the required `Action::Update` subscription
- Improved `on_action()` docs — now states it's required for litehtml/blitz engines

### Changed
- Engine view storage refactored to shared `ViewManager<V>` with HashMap for O(1) lookups
- Advanced WebView urls/titles tracking switched from Vec to HashMap for O(1) lookups
- Reduced hot-path allocations — URL/title strings moved instead of cloned, pixel buffer avoids double copy
- `on_action()` error messages now suggest the fix; doc comments name affected engines

### Fixed
- ImageInfo::blank() integer overflow — checked arithmetic with fallback to 1x1
- Widget crash on stale/invalid view index — lookups now return Option with graceful fallback
- CEF pixel buffer integer overflow — checked arithmetic prevents buffer over-read on corrupted dimensions
- Advanced widget HiDPI scaling — scroll offset and content height now scaled correctly

## [0.1.5] - 2026-03-13

### Added
- Blitz keyboard event handling — translates iced key events to blitz, enabling form interaction
- Blitz right-click, middle-click, back/forward mouse button support
- Blitz dark mode detection via `ICED_WEBVIEW_COLOR_SCHEME` env var and `GTK_THEME` fallback
- CEF mouse modifier tracking (Shift, Ctrl, Alt passed to mouse events)

### Fixed
- All engines: invalid ViewId no longer panics — `find_view` returns Option with graceful fallback
- Blitz frame comparison now uses hash instead of full pixel buffer diff
- CEF child processes (zygote, GPU, network service) left running after exit — added proper shutdown
- Litehtml selection rectangles cleared on page navigation
- Litehtml image staging deduplicates by URL (last write wins)

## [0.1.4] - 2026-02-23

### Fixed
- Servo engine view cutting off at ~2/3 screen height — viewport was never initialized after webview creation
- Servo engine not resizing when window size changes — direct rendering context resize was short-circuiting servo's internal viewport/reflow pipeline
- Advanced webview flickering with servo/cef — was using image Handle path instead of shader widget, causing texture cache churn during scrolling

## [0.1.3] - 2026-02-22

### Changed
- Default feature switched from `blitz` to `litehtml` — blitz and servo are git-only and can't be published to crates.io
- Publish workflow uses `publish.sh` to strip git-only deps before publishing

## [0.1.2] - 2026-02-22

### Added
- CEF engine — full Chromium browser via cef-rs (Tauri) behind the `cef` feature flag

## [0.1.1] - 2026-02-22

### Added
- Minimal `webview` example (just the page, no buttons or view switching)
- Shared `resolve_url()` and `is_same_page()` utilities to reduce duplication across engines
- Servo key mappings for Insert, CapsLock, NumLock, ScrollLock, Pause, PrintScreen, ContextMenu

### Fixed
- Blitz hanging on page load — `drain_resources()` was triggering full resolve + render every 10ms tick; replaced with height-change detection, a resource tick budget, and a render height cap (8192px)

### Changed
- Updated README with rendering performance comparison, Blitz known issues, and engine docs

## [0.1.0] - 2026-02-20

### Added
- Servo engine — full browser (HTML5, CSS3, JS via SpiderMonkey) as a third engine option behind the `servo` feature flag

### Changed
- Blitz deps switched from crates.io to git (DioxusLabs/blitz main) — now uses stylo 0.12, same as Servo, so both features coexist
- Updated blitz companion crates: anyrender 0.7, anyrender_vello_cpu 0.9, peniko 0.6
- Minimum Rust version bumped to 1.90

## [0.0.9] - 2026-02-20

### Added
- Blitz engine — Rust-native HTML/CSS renderer (Stylo + Taffy + Vello) with modern CSS support (flexbox, grid)

### Changed
- Default engine switched from Ultralight to Blitz
- Removed Ultralight engine and all related dependencies, build scripts, and resource handling

## [0.0.8] - 2026-02-20

### Added
- CSS `@import` resolution with recursive fetching
- CSS cache pre-loading so litehtml resolves stylesheets without network access during parsing
- Image URL resolution against stylesheet base URLs (not just page URL)

### Changed
- Stylesheet handling switched from HTML inlining to a cache-based approach via `import_css` callback
- `take_pending_images` now includes baseurl context for correct relative URL resolution
- litehtml container wrapped in `WebviewContainer` to handle CSS imports and image baseurls

## [0.0.7] - 2026-02-19

### Added
- litehtml engine with HTTP fetching, image loading, link navigation
- Example and docs for running with litehtml

## [0.0.6] - 2026-02-19

### Added
- Initial litehtml engine support as lightweight alternative to Ultralight

### Changed
- Migrated to iced 0.14

## [0.0.5] - 2025-09-27

### Added
- Generic Theme support on advanced interface

### Changed
- Relaxed trait bounds on Webview widget
- Reduced pixel format conversion overhead
- Avoided unnecessary image scaling

### Fixed
- Crash when closing view

## [0.0.4] - 2024-11-03

### Fixed
- Docs links
- Build manifest

## [0.0.3] - 2024-11-03

### Fixed
- Docs build

## [0.0.2] - 2024-11-02

### Added
- Documentation

## [0.0.1] - 2024-11-02

### Added
- Initial release — webview widget for iced, extracted from icy_browser
- Ultralight (Webkit) engine support
- Basic and advanced (multi-view) interfaces
- Example applications
