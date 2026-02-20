# Iced_webview 

[![Rust](https://github.com/franzos/iced_webview_v2/actions/workflows/rust.yml/badge.svg)](https://github.com/franzos/iced_webview_v2/actions/workflows/rust.yml) 
[![crates.io](https://img.shields.io/crates/v/iced_webview_v2.svg)](https://crates.io/crates/iced_webview_v2)

A library to embed Web views in iced applications

This library supports
- [Ultralight]/Webkit — full browser engine with JS and navigation ([license](https://ultralig.ht/pricing/))
- [litehtml] — lightweight CPU-based HTML/CSS rendering, no JS or navigation (good for static content like emails)

## Compatibility

| iced | iced_webview |
|------|--------------|
| 0.14 | 0.0.6        |
| 0.13 | 0.0.5        |

#### examples:

##### `examples/embedded_webview`
A simple example to showcase an embedded webview (uses the basic webview)
![image](https://raw.githubusercontent.com/LegitCamper/iced_webview/refs/heads/main/assets/embedded_webview.png)
```sh
cargo run --example embedded_webview --features ultralight-resources
# or with litehtml
cargo run --example embedded_webview --no-default-features --features litehtml
```

##### `examples/multi_webview`
A more advanced example that uses the advanced webview module and has two simultaneous webviews open
![image](https://raw.githubusercontent.com/LegitCamper/iced_webview/refs/heads/main/assets/multi_view.png)
```sh
cargo run --example multi_webview --features ultralight-resources
```

##### `examples/email`
Renders a table-based marketing email using litehtml — demonstrates static HTML rendering without a full browser engine
```sh
cargo run --example email --features litehtml
```

## Extra files (Resources)

Ultralight requires runtime resources. (cacert.pem, icudt67l.dat)

> You can either set the path to them with the `ULTRALIGHT_RESOURCES_DIR` env. This varible can also be set in `.cargo/config.toml`. The resouces direcory can be downloaded from [Ultralight SDK]

> Or Rust will do its best symlink the directory with `--features ultralight-resources`. If this fails please use `ULTRALIGHT_RESOURCES_DIR`

## Deployment

The samples compiled rely on dynamic libraries provided by `Ultralight`:
- `libUltralightCore.so`/`UltralightCore.dll`
- `libUltralight.so`/`Ultralight.dll`
- `libWebCore.so`/`WebCore.dll`
- `libAppCore.so`/`AppCore.dll`

These can be downloaded from the [Ultralight SDK].

> Rust will download them during build as well, but are kept inside the `target` directory.

[Ultralight]: https://ultralig.ht
[Ultralight SDK]: https://ultralig.ht/download/
[litehtml]: https://github.com/franzos/litehtml-rs

Original developer: [LegitCamper/iced_webview](https://github.com/LegitCamper/iced_webview) (Sawyer Bristol and others)
