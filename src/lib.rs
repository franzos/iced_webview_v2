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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

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

// Monotonic frame counter; lets the shader pipeline skip re-uploading unchanged pixels.
static FRAME_GENERATION: AtomicU64 = AtomicU64::new(0);

fn next_generation() -> u64 {
    FRAME_GENERATION.fetch_add(1, Ordering::Relaxed)
}

/// A frame's pixel buffer tagged with its generation, for the shader widget path.
#[cfg_attr(
    not(any(feature = "servo", feature = "cef", feature = "blitz")),
    allow(dead_code)
)]
#[derive(Clone, Debug)]
pub struct FramePixels {
    pub(crate) data: Arc<Vec<u8>>,
    pub(crate) generation: u64,
}

/// Image details for passing the view around
#[derive(Clone, Debug)]
pub struct ImageInfo {
    width: u32,
    height: u32,
    handle: Arc<OnceLock<image::Handle>>,
    raw_pixels: Arc<Vec<u8>>,
    generation: u64,
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
        // R, G, B, A: drop any trailing partial pixel from engine data.
        pixels.truncate(pixels.len() / 4 * 4);

        if let PixelFormat::Bgra = format {
            pixels
                .chunks_exact_mut(4)
                .for_each(|chunk| chunk.swap(0, 2));
        }

        Self {
            width,
            height,
            handle: Arc::new(OnceLock::new()),
            raw_pixels: Arc::new(pixels),
            generation: next_generation(),
        }
    }

    /// Get the image handle for direct rendering.
    ///
    /// Built lazily on first call: the shader widget path never needs it,
    /// so engines on that path skip the viewport-sized clone entirely.
    pub fn as_handle(&self) -> image::Handle {
        self.handle
            .get_or_init(|| {
                image::Handle::from_rgba(self.width, self.height, (*self.raw_pixels).clone())
            })
            .clone()
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
    pub fn pixels(&self) -> FramePixels {
        FramePixels {
            data: Arc::clone(&self.raw_pixels),
            generation: self.generation,
        }
    }

    fn blank(width: u32, height: u32) -> Self {
        // Fall back to 1x1 if the buffer size would overflow usize.
        let (w, h) = (width as usize)
            .checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4))
            .map_or((1u32, 1u32), |_| (width, height));

        Self {
            width: w,
            height: h,
            handle: Arc::new(OnceLock::new()),
            raw_pixels: Arc::new(vec![255; w as usize * h as usize * 4]),
            generation: next_generation(),
        }
    }
}
