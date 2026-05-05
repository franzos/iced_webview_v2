//! A library to embed web views in iced applications.
//!
//! Supports [Blitz](https://github.com/DioxusLabs/blitz) (Rust-native, modern CSS),
//! [litehtml](https://github.com/franzos/litehtml-rs) (lightweight, CPU-based), and
//! [Servo](https://servo.org/) (full browser: HTML5, CSS3, JS).
//!
//! Has two separate widgets: Basic, and Advanced.
//! The basic widget is simple to implement — use abstractions like `CloseCurrent` and `ChangeView`.
//! The advanced widget gives you direct `ViewId` control for multiple simultaneous views.
//!
//! # Basic usage
//!
//! ```rust,ignore
//! enum Message {
//!    WebView(iced_webview::Action),
//!    Update,
//! }
//!
//! struct State {
//!    webview: iced_webview::WebView<iced_webview::Blitz, Message>,
//! }
//! ```
//!
//! Then call the usual `view/update` methods — see
//! [examples](https://github.com/franzos/iced_webview_v2/tree/main/examples) for full working code.
//!
use std::sync::Arc;

use iced::widget::image;

/// Engine Trait and Engine implementations
pub mod engines;
pub use engines::{Engine, PageType, PixelFormat, ViewId};

mod webview;
pub use basic::{Action, WebView};
pub use webview::{advanced, basic};

#[cfg(feature = "blitz")]
pub use engines::blitz::Blitz;

#[cfg(feature = "litehtml")]
pub use engines::litehtml::Litehtml;

#[cfg(feature = "servo")]
pub use engines::servo::Servo;

#[cfg(feature = "cef")]
pub use engines::cef_engine::{cef_subprocess_check, Cef};

pub(crate) mod util;

#[cfg(any(feature = "litehtml", feature = "blitz"))]
pub(crate) mod fetch;

/// Image details for passing the view around
#[derive(Clone, Debug)]
pub struct ImageInfo {
    width: u32,
    height: u32,
    handle: image::Handle,
    raw_pixels: Arc<Vec<u8>>,
}

impl Default for ImageInfo {
    fn default() -> Self {
        Self::blank(Self::WIDTH, Self::HEIGHT)
    }
}

impl ImageInfo {
    // The default dimensions
    const WIDTH: u32 = 800;
    const HEIGHT: u32 = 800;

    #[allow(dead_code)]
    fn new(mut pixels: Vec<u8>, format: PixelFormat, width: u32, height: u32) -> Self {
        // R, G, B, A
        assert_eq!(pixels.len() % 4, 0);

        if let PixelFormat::Bgra = format {
            pixels.chunks_mut(4).for_each(|chunk| chunk.swap(0, 2));
        }

        let raw_pixels = Arc::new(pixels);
        Self {
            width,
            height,
            handle: image::Handle::from_rgba(width, height, (*raw_pixels).clone()),
            raw_pixels,
        }
    }

    /// Construct an `ImageInfo` for the shader-widget rendering path,
    /// skipping the `image::Handle` allocation. Saves a viewport-sized
    /// clone per frame for engines that never read `handle`.
    #[allow(dead_code)]
    pub(crate) fn from_shader_pixels(pixels: Vec<u8>, width: u32, height: u32) -> Self {
        debug_assert_eq!(pixels.len() % 4, 0);
        Self {
            width,
            height,
            // 1×1 placeholder; the shader widget path doesn't read this.
            handle: image::Handle::from_rgba(1, 1, vec![0u8; 4]),
            raw_pixels: Arc::new(pixels),
        }
    }

    /// Get the image handle for direct rendering.
    pub fn as_handle(&self) -> image::Handle {
        self.handle.clone()
    }

    /// Image width.
    pub fn image_width(&self) -> u32 {
        self.width
    }

    /// Image height.
    pub fn image_height(&self) -> u32 {
        self.height
    }

    /// Raw RGBA pixel data for direct GPU upload (shader widget path).
    pub fn pixels(&self) -> Arc<Vec<u8>> {
        Arc::clone(&self.raw_pixels)
    }

    fn blank(width: u32, height: u32) -> Self {
        let (w, h) = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .map_or((1u32, 1u32), |_| (width, height));

        let pixels = vec![255; (w as usize * h as usize) * 4];
        let raw_pixels = Arc::new(pixels.clone());
        Self {
            width: w,
            height: h,
            handle: image::Handle::from_rgba(w, h, pixels),
            raw_pixels,
        }
    }
}
