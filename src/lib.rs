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

pub(crate) mod util;

#[cfg(any(feature = "litehtml", feature = "blitz"))]
pub(crate) mod fetch;

/// Image details for passing the view around
#[derive(Clone, Debug)]
pub struct ImageInfo {
    width: u32,
    height: u32,
    handle: image::Handle,
}

impl Default for ImageInfo {
    fn default() -> Self {
        let pixels = vec![255; (Self::WIDTH as usize * Self::HEIGHT as usize) * 4];
        Self {
            width: Self::WIDTH,
            height: Self::HEIGHT,
            handle: image::Handle::from_rgba(Self::WIDTH, Self::HEIGHT, pixels),
        }
    }
}

impl ImageInfo {
    // The default dimensions
    const WIDTH: u32 = 800;
    const HEIGHT: u32 = 800;

    fn new(mut pixels: Vec<u8>, format: PixelFormat, width: u32, height: u32) -> Self {
        // R, G, B, A
        assert_eq!(pixels.len() % 4, 0);

        if let PixelFormat::Bgra = format {
            pixels.chunks_mut(4).for_each(|chunk| chunk.swap(0, 2));
        }

        Self {
            width,
            height,
            handle: image::Handle::from_rgba(width, height, pixels),
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

    fn blank(width: u32, height: u32) -> Self {
        let pixels = vec![255; (width as usize * height as usize) * 4];
        Self {
            width,
            height,
            handle: image::Handle::from_rgba(width, height, pixels),
        }
    }
}
