use std::collections::HashMap;

use crate::ImageInfo;
use iced::keyboard;
use iced::mouse::{self, Interaction};
use iced::Point;
use iced::Size;

/// A Blitz implementation of Engine (Stylo + Taffy + Vello)
#[cfg(feature = "blitz")]
pub mod blitz;

/// A litehtml implementation of Engine for HTML rendering
#[cfg(feature = "litehtml")]
pub mod litehtml;

/// A Servo implementation of Engine (full browser: HTML5, CSS3, JS)
#[cfg(feature = "servo")]
pub mod servo;

/// Creation of new pages to be of a html type or a url
#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum PageType {
    /// Allows visiting Url web pages
    Url(String),
    /// Allows custom html web pages
    Html(String),
}

/// Enables browser engines to display their images in different formats
pub enum PixelFormat {
    /// RGBA
    Rgba,
    /// BGRA
    Bgra,
}

/// Alias of usize used for controlling specific views
/// Only used by advanced to get views, basic simply uses u32
pub type ViewId = usize;

/// Trait to handle multiple browser engines
/// Currently only supports cpu renders via pixel_buffer
/// Passing a View id that does not exist will cause a panic
pub trait Engine {
    /// Used to do work in the actual browser engine
    fn update(&mut self);
    /// Request a new render pass from the engine
    fn render(&mut self, size: Size<u32>);
    /// Request that the browser engine rerender a specific view that may have been updated
    fn request_render(&mut self, id: ViewId, size: Size<u32>);
    /// Creates new a new (possibly blank) view and returns the ViewId to interact with it
    fn new_view(&mut self, size: Size<u32>, content: Option<PageType>) -> ViewId;
    /// Removes desired view
    fn remove_view(&mut self, id: ViewId);
    /// Whether a view with this id currently exists.
    fn has_view(&self, _id: ViewId) -> bool {
        false
    }

    /// Focuses webview
    fn focus(&mut self);
    /// Unfocuses webview
    fn unfocus(&self);
    /// Resizes webview
    fn resize(&mut self, size: Size<u32>);
    /// Set the display scale factor for HiDPI rendering. Default is no-op.
    fn set_scale_factor(&mut self, _scale: f32) {}

    /// Whether this engine can fetch and render URLs natively.
    /// Engines that return `false` rely on the webview layer to fetch HTML.
    fn handles_urls(&self) -> bool {
        true
    }

    /// lets the engine handle keyboard events
    fn handle_keyboard_event(&mut self, id: ViewId, event: keyboard::Event);
    /// lets the engine handle mouse events
    fn handle_mouse_event(&mut self, id: ViewId, point: Point, event: mouse::Event);
    /// Handles scrolling on view
    fn scroll(&mut self, id: ViewId, delta: mouse::ScrollDelta);

    /// Go to a specific page type
    fn goto(&mut self, id: ViewId, page_type: PageType);
    /// Refresh specific view
    fn refresh(&mut self, id: ViewId);
    /// Moves forward on view
    fn go_forward(&mut self, id: ViewId);
    /// Moves back on view
    fn go_back(&mut self, id: ViewId);

    /// Gets current url from view
    fn get_url(&self, id: ViewId) -> String;
    /// Gets current title from view
    fn get_title(&self, id: ViewId) -> String;
    /// Gets current cursor status from view
    fn get_cursor(&self, id: ViewId) -> Interaction;
    /// Gets CPU-rendered webview
    fn get_view(&self, id: ViewId) -> &ImageInfo;

    /// Current vertical scroll offset (logical pixels).
    fn get_scroll_y(&self, _id: ViewId) -> f32 {
        0.0
    }

    /// Total content height (logical pixels). Zero means the engine manages scrolling.
    fn get_content_height(&self, _id: ViewId) -> f32 {
        0.0
    }

    /// Gets the currently selected text from a view, if any.
    fn get_selected_text(&self, _id: ViewId) -> Option<String> {
        None
    }

    /// Selection highlight rectangles for overlay rendering.
    /// Returns `[x, y, width, height]` in logical coordinates, scroll-adjusted.
    fn get_selection_rects(&self, _id: ViewId) -> &[[f32; 4]] {
        &[]
    }

    /// Take the last anchor click URL from a view, if any.
    /// Called after mouse events to detect link navigation.
    fn take_anchor_click(&mut self, _id: ViewId) -> Option<String> {
        None
    }

    /// Scroll to a named fragment (e.g. `"section2"` for `#section2`).
    /// Returns `true` if the fragment was found and the view scrolled.
    fn scroll_to_fragment(&mut self, _id: ViewId, _fragment: &str) -> bool {
        false
    }

    /// Return image URLs discovered during layout that still need fetching.
    /// Each entry is `(view_id, raw_src, baseurl, redraw_on_ready)` — the
    /// consumer resolves URLs against baseurl and threads `redraw_on_ready`
    /// back through `load_image_from_bytes`.
    fn take_pending_images(&mut self) -> Vec<(ViewId, String, String, bool)> {
        Vec::new()
    }

    /// Pre-load a CSS cache into a view's container so `import_css` can
    /// resolve stylesheets without network access during parsing.
    fn set_css_cache(&mut self, _id: ViewId, _cache: HashMap<String, String>) {}

    /// Inject fetched image bytes into a view's container, keyed by the
    /// raw `src` value from the HTML. When `redraw_on_ready` is true, the
    /// image doesn't affect layout (CSS background or `<img>` with explicit
    /// dimensions) so `doc.render()` can be skipped — only a redraw is needed.
    fn load_image_from_bytes(
        &mut self,
        _id: ViewId,
        _url: &str,
        _bytes: &[u8],
        _redraw_on_ready: bool,
    ) {
    }

    /// Flush all staged images into the document and redraw.
    /// Called when all in-flight image fetches have completed so the
    /// full batch is processed in a single redraw.
    fn flush_staged_images(&mut self, _id: ViewId, _size: Size<u32>) {}

    /// Return all active view IDs.
    fn view_ids(&self) -> Vec<ViewId> {
        Vec::new()
    }
}
