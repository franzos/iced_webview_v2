# Changelog

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
