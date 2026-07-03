use std::cell::RefCell;
use std::collections::HashMap;

use iced::keyboard;
use iced::mouse::{self, Interaction};
use iced::{Point, Size};
use url::Url;

use super::{Engine, PageType, PixelFormat, ViewId, ViewManager};
use crate::ImageInfo;

use litehtml::pixbuf::PixbufContainer;
use litehtml::selection::Selection;
use litehtml::{
    css_escape_ident, BackgroundLayer, BorderRadiuses, Borders, Color, ConicGradient, Document,
    DocumentContainer, DrawContext, FontDescription, FontHandle, FontMetrics, LinearGradient,
    ListMarker, MediaFeatures, Position, RadialGradient, TextTransform,
};

/// Cap for content-driven pixel buffer height; hostile CSS could otherwise
/// drive multi-gigabyte allocations via huge layout heights.
const MAX_RENDER_HEIGHT_PX: u32 = 32_768;

/// Wrapper around `PixbufContainer` that handles CSS import resolution
/// and image baseurl tracking, mirroring litehtml-rs's `BrowseContainer`.
struct WebviewContainer {
    inner: PixbufContainer,
    base_url: String,
    caption: String,
    css_cache: RefCell<HashMap<String, String>>,
    /// Maps raw image src → baseurl passed by litehtml, so image fetches
    /// can resolve relative URLs against the correct context (stylesheet
    /// URL, not the page URL).
    image_baseurls: RefCell<HashMap<String, String>>,
}

impl WebviewContainer {
    fn new(width: u32, height: u32, scale: f32) -> Self {
        Self {
            inner: PixbufContainer::new_with_scale(width, height, scale),
            base_url: String::new(),
            caption: String::new(),
            css_cache: RefCell::new(HashMap::new()),
            image_baseurls: RefCell::new(HashMap::new()),
        }
    }

    fn inner(&self) -> &PixbufContainer {
        &self.inner
    }

    fn inner_mut(&mut self) -> &mut PixbufContainer {
        &mut self.inner
    }

    fn set_css_cache(&self, cache: HashMap<String, String>) {
        *self.css_cache.borrow_mut() = cache;
    }

    /// Resolve a URL against a given base, falling back to self.base_url.
    fn resolve_against(&self, href: &str, baseurl: &str) -> Option<Url> {
        // Already absolute
        if let Ok(u) = Url::parse(href) {
            return Some(u);
        }
        // Resolve against the provided base context (e.g. stylesheet URL)
        if !baseurl.is_empty() {
            if let Ok(base) = Url::parse(baseurl) {
                if let Ok(u) = base.join(href) {
                    return Some(u);
                }
            }
        }
        // Fall back to page base URL
        if !self.base_url.is_empty() {
            if let Ok(base) = Url::parse(&self.base_url) {
                return base.join(href).ok();
            }
        }
        None
    }
}

// Delegate everything to inner, override import_css, set_base_url, load_image
impl DocumentContainer for WebviewContainer {
    fn create_font(&mut self, descr: &FontDescription) -> (FontHandle, FontMetrics) {
        self.inner.create_font(descr)
    }
    fn delete_font(&mut self, font: FontHandle) {
        self.inner.delete_font(font);
    }
    fn text_width(&self, text: &str, font: FontHandle) -> f32 {
        self.inner.text_width(text, font)
    }
    fn draw_text(
        &mut self,
        hdc: DrawContext,
        text: &str,
        font: FontHandle,
        color: Color,
        pos: Position,
    ) {
        self.inner.draw_text(hdc, text, font, color, pos);
    }
    fn draw_list_marker(&mut self, hdc: DrawContext, marker: &ListMarker) {
        self.inner.draw_list_marker(hdc, marker);
    }
    fn load_image(&mut self, src: &str, baseurl: &str, redraw_on_ready: bool) {
        // Store the baseurl context so image fetches can resolve correctly
        if !baseurl.is_empty() {
            self.image_baseurls
                .borrow_mut()
                .insert(src.to_string(), baseurl.to_string());
        }
        self.inner.load_image(src, baseurl, redraw_on_ready);
    }
    fn get_image_size(&self, src: &str, baseurl: &str) -> litehtml::Size {
        self.inner.get_image_size(src, baseurl)
    }
    fn draw_image(&mut self, hdc: DrawContext, layer: &BackgroundLayer, url: &str, base_url: &str) {
        self.inner.draw_image(hdc, layer, url, base_url);
    }
    fn draw_solid_fill(&mut self, hdc: DrawContext, layer: &BackgroundLayer, color: Color) {
        self.inner.draw_solid_fill(hdc, layer, color);
    }
    fn draw_linear_gradient(
        &mut self,
        hdc: DrawContext,
        layer: &BackgroundLayer,
        gradient: &LinearGradient,
    ) {
        self.inner.draw_linear_gradient(hdc, layer, gradient);
    }
    fn draw_radial_gradient(
        &mut self,
        hdc: DrawContext,
        layer: &BackgroundLayer,
        gradient: &RadialGradient,
    ) {
        self.inner.draw_radial_gradient(hdc, layer, gradient);
    }
    fn draw_conic_gradient(
        &mut self,
        hdc: DrawContext,
        layer: &BackgroundLayer,
        gradient: &ConicGradient,
    ) {
        self.inner.draw_conic_gradient(hdc, layer, gradient);
    }
    fn draw_borders(
        &mut self,
        hdc: DrawContext,
        borders: &Borders,
        draw_pos: Position,
        root: bool,
    ) {
        self.inner.draw_borders(hdc, borders, draw_pos, root);
    }
    fn set_caption(&mut self, caption: &str) {
        self.caption = caption.to_string();
        self.inner.set_caption(caption);
    }
    fn set_base_url(&mut self, base_url: &str) {
        // Update our stored base URL for resolve_against fallback
        self.base_url = base_url.to_string();
        self.inner.set_base_url(base_url);
    }
    fn on_anchor_click(&mut self, url: &str) {
        self.inner.on_anchor_click(url);
    }
    fn set_cursor(&mut self, cursor: &str) {
        self.inner.set_cursor(cursor);
    }
    fn transform_text(&self, text: &str, tt: TextTransform) -> String {
        self.inner.transform_text(text, tt)
    }
    fn import_css(&self, url: &str, baseurl: &str) -> (String, Option<String>) {
        // Resolve against the baseurl parameter (stylesheet context)
        let resolved = match self.resolve_against(url, baseurl) {
            Some(u) => u,
            None => return (String::new(), None),
        };
        let key = resolved.to_string();
        if let Some(cached) = self.css_cache.borrow().get(&key) {
            return (cached.clone(), Some(key));
        }
        // Not in cache — return empty (the CSS wasn't pre-fetched)
        (String::new(), None)
    }
    fn set_clip(&mut self, pos: Position, radius: BorderRadiuses) {
        self.inner.set_clip(pos, radius);
    }
    fn del_clip(&mut self) {
        self.inner.del_clip();
    }
    fn get_viewport(&self) -> Position {
        self.inner.get_viewport()
    }
    fn get_media_features(&self) -> MediaFeatures {
        self.inner.get_media_features()
    }
}

/// Persistent document and selection state for a view.
///
/// # Safety
///
/// The `doc` field borrows the `WebviewContainer` owned by the parent
/// `LitehtmlView`, via `container_ptr`. `doc_state` is declared before
/// `_container` so it drops first, and is set to `None` before the container
/// is resized destructively or dropped. While a document exists, every
/// container access goes through `container_ptr` (see
/// `LitehtmlView::container`), never through a fresh `Box`-derived reference.
type MeasureFn = Box<dyn Fn(&str, FontHandle) -> f32>;

struct DocumentState {
    doc: Document<'static>,
    measure: MeasureFn,
    selection: Selection<'static>,
}

struct LitehtmlView {
    // IMPORTANT: doc_state must be declared before _container so it drops first.
    doc_state: Option<DocumentState>,
    /// Owns the container allocation; never accessed directly after
    /// construction. All access goes through `container_ptr`.
    _container: Box<WebviewContainer>,
    container_ptr: *mut WebviewContainer,
    html: String,
    url: String,
    title: String,
    cursor: Interaction,
    last_frame: ImageInfo,
    needs_render: bool,
    /// Fetched image bytes waiting to be flushed into the container.
    /// Accumulated between render cycles so multiple images cause only
    /// one document rebuild instead of one per image.
    staged_images: Vec<(String, Vec<u8>, bool)>,
    /// Selection highlight rects in logical coords, scroll-adjusted.
    /// Drawn as iced quads by the widget so the base image Handle stays stable.
    selection_rects: Vec<[f32; 4]>,
    scroll_y: f32,
    content_height: f32,
    size: Size<u32>,
    drag_origin: Option<(f32, f32)>,
    drag_active: bool,
}

impl LitehtmlView {
    // SAFETY invariant for both accessors and for the `&'static mut` handed to
    // `Document::from_html` in `rebuild_document`:
    //   - `container_ptr` points into the heap allocation owned by
    //     `_container`, created once at construction; moving `LitehtmlView`
    //     moves the Box value but not the allocation, so it stays valid.
    //   - All container access while a `Document` exists derives from this
    //     one raw pointer, never from `_container`, so no fresh Box-derived
    //     unique reference invalidates the document's borrow.
    //   - Everything is single-threaded, and the engine never touches the
    //     container while a document callback is on the stack, so no two
    //     pointer-derived references are used simultaneously.
    // Miri cannot verify this: litehtml is C++ FFI.
    fn container(&self) -> &WebviewContainer {
        unsafe { &*self.container_ptr }
    }

    fn container_mut(&mut self) -> &mut WebviewContainer {
        unsafe { &mut *self.container_ptr }
    }
}

/// CPU-based HTML rendering engine backed by litehtml.
///
/// No URL navigation, no keyboard input, no JavaScript.
/// Uses `litehtml::pixbuf::PixbufContainer` for software rasterization.
pub struct Litehtml {
    views: ViewManager<LitehtmlView>,
    scale_factor: f32,
}

impl Default for Litehtml {
    fn default() -> Self {
        Self {
            views: ViewManager::default(),
            scale_factor: 1.0,
        }
    }
}

/// Build a persistent Document for the view, storing it alongside its
/// text-measurement closure and a fresh Selection.
///
/// Drops any existing document state first (releasing the container borrow),
/// then resizes the container, creates a new Document, and renders the layout.
fn rebuild_document(view: &mut LitehtmlView) {
    view.doc_state = None;

    // Flush any staged images while doc_state is None (safe — no Document
    // holds a borrow of the container).
    if !view.staged_images.is_empty() {
        for (url, bytes, _) in std::mem::take(&mut view.staged_images) {
            view.container_mut()
                .inner_mut()
                .load_image_data(&url, &bytes);
        }
    }

    let w = view.size.width;
    let h = view.size.height;

    if w == 0 || h == 0 || view.html.is_empty() {
        return;
    }

    // Pass 1: use a tall viewport so CSS `100vh` doesn't cap content height.
    let layout_h = h.max(10_000);
    view.container_mut().inner_mut().resize(w, layout_h);
    view.container_mut().caption.clear();

    // Capture the text measurement closure before borrowing the container
    let measure = view.container().inner().text_measure_fn();

    // The litehtml Document<'a> is invariant over 'a and takes `&'a mut`, so
    // safe self-referential crates (ouroboros, self_cell) can't express this;
    // the lifetime is extended to 'static manually.
    //
    // SAFETY: see `LitehtmlView::container`. `doc_state` is None here, so no
    // other document borrows the container, and this reference derives from
    // `container_ptr` like all other container access.
    let container_ref: &'static mut WebviewContainer = unsafe { &mut *view.container_ptr };

    match Document::from_html(&view.html, container_ref, None, None) {
        Err(e) => {
            log::warn!("litehtml: from_html failed: {e:?}");
        }
        Ok(mut doc) => {
            let _ = doc.render(w as f32);
            let measured = doc.height();

            // Pass 2: if content overflows the layout viewport, re-layout so
            // `100vh` covers the full content and overflow clips don't cut it off.
            if measured > layout_h as f32 {
                let final_h = (measured.ceil() as u32).min(MAX_RENDER_HEIGHT_PX);

                // Drop the document BEFORE resizing so its borrow of the
                // container has ended.
                drop(doc);

                view.container_mut().inner_mut().resize(w, final_h);
                let measure2 = view.container().inner().text_measure_fn();

                // SAFETY: same invariant as pass 1 above; the pass-1 document
                // was dropped.
                let container_ref2: &'static mut WebviewContainer =
                    unsafe { &mut *view.container_ptr };

                match Document::from_html(&view.html, container_ref2, None, None) {
                    Err(e) => {
                        log::warn!("litehtml: from_html pass 2 failed: {e:?}");
                    }
                    Ok(mut doc2) => {
                        let _ = doc2.render(w as f32);
                        // Clamp so scrolling stays within the rendered buffer.
                        view.content_height = doc2.height().min(MAX_RENDER_HEIGHT_PX as f32);

                        let selection: Selection<'static> = Selection::new();
                        view.doc_state = Some(DocumentState {
                            doc: doc2,
                            measure: Box::new(measure2),
                            selection,
                        });
                    }
                }
            } else {
                view.content_height = measured;

                let selection: Selection<'static> = Selection::new();
                view.doc_state = Some(DocumentState {
                    doc,
                    measure: Box::new(measure),
                    selection,
                });
            }
        }
    }

    view.title = view.container().caption.clone();
}

/// Draw the document into the pixel buffer and capture `last_frame`.
///
/// Resizes the container to fit the full content height, disables CSS
/// overflow clips, draws, then captures the pixels. All container access
/// goes through `container_ptr` (see `LitehtmlView::container`), so it does
/// not invalidate the live document's borrow.
fn capture_frame(view: &mut LitehtmlView) {
    let w = view.size.width;
    // Clamp: content height is attacker-influenced (hostile CSS), so cap the
    // buffer size. `as u32` saturates, so huge/NaN heights are handled too.
    let full_h = (view.content_height.ceil() as u32).clamp(1, MAX_RENDER_HEIGHT_PX);

    view.container_mut().inner_mut().resize(w, full_h);
    view.container_mut()
        .inner_mut()
        .set_ignore_overflow_clips(true);

    if let Some(ref mut ds) = view.doc_state {
        let clip = Position {
            x: 0.0,
            y: 0.0,
            width: w as f32,
            height: full_h as f32,
        };
        ds.doc.draw(DrawContext(0), 0.0, 0.0, Some(clip));
    }

    view.container_mut()
        .inner_mut()
        .set_ignore_overflow_clips(false);

    let phys_w = view.container().inner().width();
    let phys_h = view.container().inner().height();
    let mut pixels = view.container().inner().pixels().to_vec();
    unpremultiply_rgba(&mut pixels);
    view.last_frame = ImageInfo::new(pixels, PixelFormat::Rgba, phys_w, phys_h);
    view.needs_render = false;
}

/// Render the full document into the pixel buffer and update `last_frame`.
///
/// The buffer covers the entire content height so the widget can scroll
/// by offsetting the draw position — no re-render needed per scroll.
fn draw_view(view: &mut LitehtmlView) {
    capture_frame(view);
}

/// Flush staged image bytes into the container and redraw.
///
/// Drops the document first to release its borrow on the container, loads
/// the image data, then rebuilds the document from the stored HTML.
///
/// The rebuild cost is acceptable: `Document::from_html` + `render` is fast
/// for typical HTML, and this path only runs when async image fetches complete.
fn flush_images_and_redraw(view: &mut LitehtmlView) {
    if view.staged_images.is_empty() {
        return;
    }

    let w = view.size.width;
    if w == 0 || view.size.height == 0 {
        return;
    }

    // Drop the document to release its borrow on the container.
    view.doc_state = None;

    // Now safe to mutate the container — no Document holds a borrow.
    for (url, bytes, _) in std::mem::take(&mut view.staged_images) {
        view.container_mut()
            .inner_mut()
            .load_image_data(&url, &bytes);
    }

    // Rebuild the document from stored HTML (includes render + draw).
    rebuild_document(view);
    capture_frame(view);
}

/// Main render entry point: rebuilds the document if needed, then draws.
fn render_view(view: &mut LitehtmlView) {
    let w = view.size.width;
    let h = view.size.height;

    if w == 0 || h == 0 {
        return;
    }

    if view.html.is_empty() {
        let phys_w = view.container().inner().width();
        let phys_h = view.container().inner().height();
        view.last_frame = ImageInfo::blank(phys_w, phys_h);
        view.needs_render = false;
        return;
    }

    if view.doc_state.is_none() {
        rebuild_document(view);
        draw_view(view);
    } else if !view.staged_images.is_empty() {
        // Document exists — inject images, re-layout, and draw in one
        // pass so the container size stays consistent throughout.
        flush_images_and_redraw(view);
    } else {
        draw_view(view);
    }
}

/// Convert premultiplied-alpha RGBA pixels to straight alpha, in place.
///
/// litehtml's pixbuf backend (tiny-skia) stores premultiplied RGBA, but
/// iced's `image::Handle::from_rgba` expects straight (unpremultiplied) alpha.
fn unpremultiply_rgba(pixels: &mut [u8]) {
    for chunk in pixels.chunks_exact_mut(4) {
        let a = chunk[3] as u32;
        for c in &mut chunk[..3] {
            *c = (*c as u32 * 255 + a / 2)
                .checked_div(a)
                .unwrap_or(0)
                .min(255) as u8;
        }
    }
}

/// Map a CSS cursor value from litehtml to an iced mouse interaction.
fn css_cursor_to_interaction(cursor: &str) -> Interaction {
    match cursor {
        "pointer" => Interaction::Pointer,
        "text" => Interaction::Text,
        "crosshair" => Interaction::Crosshair,
        "grab" => Interaction::Grab,
        "grabbing" => Interaction::Grabbing,
        "not-allowed" | "no-drop" => Interaction::NotAllowed,
        "col-resize" | "ew-resize" => Interaction::ResizingHorizontally,
        "row-resize" | "ns-resize" => Interaction::ResizingVertically,
        _ => Interaction::Idle,
    }
}

/// Store selection rectangles in document coordinates.
/// The widget layer applies the scroll offset when drawing.
fn update_selection_rects(view: &mut LitehtmlView) {
    view.selection_rects.clear();
    if let Some(ref state) = view.doc_state {
        for r in state.selection.rectangles() {
            view.selection_rects.push([r.x, r.y, r.width, r.height]);
        }
    }
}

impl Engine for Litehtml {
    fn handles_urls(&self) -> bool {
        false
    }

    fn update(&mut self) {
        // No-op: litehtml has no async work or background tasks.
    }

    fn render(&mut self) {
        for view in self.views.values_mut() {
            if view.needs_render {
                render_view(view);
            }
        }
    }

    fn request_render(&mut self, id: ViewId) {
        let Some(view) = self.views.get_mut(id) else {
            return;
        };
        if view.needs_render {
            render_view(view);
        }
    }

    fn flush_staged_images(&mut self, id: ViewId, _size: Size<u32>) {
        let Some(view) = self.views.get_mut(id) else {
            return;
        };
        if !view.staged_images.is_empty() {
            render_view(view);
        }
    }

    fn new_view(&mut self, size: Size<u32>, content: Option<PageType>) -> ViewId {
        let w = size.width.max(1);
        let h = size.height.max(1);

        let html = match &content {
            Some(PageType::Html(html)) => html.clone(),
            _ => String::new(),
        };
        let url = match &content {
            Some(PageType::Url(url)) => url.clone(),
            _ => String::new(),
        };

        let mut container = Box::new(WebviewContainer::new(w, h, self.scale_factor));
        let container_ptr: *mut WebviewContainer = &mut *container;

        let mut view = LitehtmlView {
            doc_state: None,
            _container: container,
            container_ptr,
            html,
            url,
            title: String::new(),
            cursor: Interaction::Idle,
            last_frame: ImageInfo::blank(w, h),
            needs_render: true,
            staged_images: Vec::new(),
            selection_rects: Vec::new(),
            scroll_y: 0.0,
            content_height: 0.0,
            size,
            drag_origin: None,
            drag_active: false,
        };

        render_view(&mut view);
        self.views.insert(view)
    }

    fn remove_view(&mut self, id: ViewId) {
        self.views.remove(id);
    }

    fn has_view(&self, id: ViewId) -> bool {
        self.views.contains(id)
    }

    fn view_ids(&self) -> Vec<ViewId> {
        self.views.keys()
    }

    fn resize(&mut self, size: Size<u32>) {
        for view in self.views.values_mut() {
            view.doc_state = None;

            view.size = size;
            view.needs_render = true;
        }
    }

    fn set_scale_factor(&mut self, scale: f32) {
        if (self.scale_factor - scale).abs() < f32::EPSILON {
            return;
        }
        self.scale_factor = scale;
        for view in self.views.values_mut() {
            view.doc_state = None;

            let size = view.size;
            view.container_mut()
                .inner_mut()
                .resize_with_scale(size.width, size.height, scale);
            view.needs_render = true;
        }
    }

    fn handle_keyboard_event(&mut self, _id: ViewId, _event: keyboard::Event) {
        // No-op: litehtml has no keyboard interaction.
    }

    fn handle_mouse_event(&mut self, id: ViewId, point: Point, event: mouse::Event) {
        match event {
            mouse::Event::WheelScrolled { delta } => {
                self.scroll(id, delta);
            }
            mouse::Event::ButtonPressed(mouse::Button::Left) => {
                let Some(view) = self.views.get_mut(id) else {
                    return;
                };
                view.drag_origin = Some((point.x, point.y));
                view.drag_active = false;
                if let Some(ref mut state) = view.doc_state {
                    let doc_y = point.y + view.scroll_y;
                    state.doc.on_lbutton_down(point.x, doc_y, point.x, point.y);
                    state.selection.clear();
                }
                view.selection_rects.clear();
            }
            mouse::Event::CursorMoved { .. } => {
                let Some(view) = self.views.get_mut(id) else {
                    return;
                };

                // Notify litehtml of mouse movement for :hover and cursor updates
                if let Some(ref mut state) = view.doc_state {
                    let doc_y = point.y + view.scroll_y;
                    state.doc.on_mouse_over(point.x, doc_y, point.x, point.y);
                }
                view.cursor = css_cursor_to_interaction(view.container().inner().cursor());

                if let Some((ox, oy)) = view.drag_origin {
                    let dx = point.x - ox;
                    let dy = point.y - oy;

                    if !view.drag_active && (dx * dx + dy * dy).sqrt() >= 4.0 {
                        view.drag_active = true;
                        if let Some(ref mut state) = view.doc_state {
                            let doc_y = oy + view.scroll_y;
                            state.selection.start_at(
                                &state.doc,
                                &*state.measure,
                                ox,
                                doc_y,
                                ox,
                                oy,
                            );
                        }
                    }

                    if view.drag_active {
                        if let Some(ref mut state) = view.doc_state {
                            let doc_y = point.y + view.scroll_y;
                            state.selection.extend_to(
                                &state.doc,
                                &*state.measure,
                                point.x,
                                doc_y,
                                point.x,
                                point.y,
                            );
                        }
                        update_selection_rects(view);
                    }
                }
            }
            mouse::Event::ButtonReleased(mouse::Button::Left) => {
                let Some(view) = self.views.get_mut(id) else {
                    return;
                };
                let was_dragging = view.drag_active;
                view.drag_active = false;
                view.drag_origin = None;

                // Always notify litehtml to clear :active pseudo-class state
                if let Some(ref mut state) = view.doc_state {
                    let doc_y = point.y + view.scroll_y;
                    state.doc.on_lbutton_up(point.x, doc_y, point.x, point.y);
                }
                // Discard anchor clicks produced during text selection.
                if was_dragging {
                    view.container_mut().inner_mut().take_anchor_click();
                }
            }
            mouse::Event::CursorLeft => {
                if let Some(view) = self.views.get_mut(id) {
                    if let Some(ref mut state) = view.doc_state {
                        state.doc.on_mouse_leave();
                    }
                    view.cursor = Interaction::Idle;
                }
            }
            _ => {}
        }
    }

    fn scroll(&mut self, id: ViewId, delta: mouse::ScrollDelta) {
        let Some(view) = self.views.get_mut(id) else {
            return;
        };
        match delta {
            mouse::ScrollDelta::Lines { y, .. } => {
                view.scroll_y -= y * 40.0;
            }
            mouse::ScrollDelta::Pixels { y, .. } => {
                view.scroll_y -= y;
            }
        }
        let max_scroll = (view.content_height - view.size.height as f32).max(0.0);
        view.scroll_y = view.scroll_y.clamp(0.0, max_scroll);
    }

    fn goto(&mut self, id: ViewId, page_type: PageType) {
        let Some(view) = self.views.get_mut(id) else {
            return;
        };
        match page_type {
            PageType::Html(html) => {
                view.doc_state = None;
                // Clear image state from the previous page so stale fetches
                // don't interfere and new images are discovered fresh.
                view.staged_images.clear();
                view.container_mut().inner_mut().clear_pending_images();
                // Clear image baseurls from the previous page
                view.container().image_baseurls.borrow_mut().clear();
                view.selection_rects.clear();

                view.html = html;
                view.scroll_y = 0.0;
                view.needs_render = true;
            }
            PageType::Url(url) => {
                view.container_mut().base_url = url.clone();
                view.url = url;
            }
        }
    }

    fn refresh(&mut self, id: ViewId) {
        let Some(view) = self.views.get_mut(id) else {
            return;
        };
        view.doc_state = None;

        view.needs_render = true;
    }

    fn go_forward(&mut self, _id: ViewId) {
        // No-op: no navigation history.
    }

    fn go_back(&mut self, _id: ViewId) {
        // No-op: no navigation history.
    }

    fn get_url(&self, id: ViewId) -> String {
        let Some(view) = self.views.get(id) else {
            return "about:blank".to_string();
        };
        if view.url.is_empty() {
            "about:blank".to_string()
        } else {
            view.url.clone()
        }
    }

    fn get_title(&self, id: ViewId) -> String {
        self.views
            .get(id)
            .map(|v| v.title.clone())
            .unwrap_or_default()
    }

    fn get_cursor(&self, id: ViewId) -> Interaction {
        self.views
            .get(id)
            .map(|v| v.cursor)
            .unwrap_or(Interaction::Idle)
    }

    fn get_view(&self, id: ViewId) -> &ImageInfo {
        static BLANK: std::sync::LazyLock<ImageInfo> = std::sync::LazyLock::new(ImageInfo::default);
        self.views.get(id).map(|v| &v.last_frame).unwrap_or(&BLANK)
    }

    fn get_scroll_y(&self, id: ViewId) -> f32 {
        self.views.get(id).map(|v| v.scroll_y).unwrap_or(0.0)
    }

    fn get_content_height(&self, id: ViewId) -> f32 {
        self.views.get(id).map(|v| v.content_height).unwrap_or(0.0)
    }

    fn get_selected_text(&self, id: ViewId) -> Option<String> {
        self.views
            .get(id)?
            .doc_state
            .as_ref()?
            .selection
            .selected_text()
    }

    fn get_selection_rects(&self, id: ViewId) -> &[[f32; 4]] {
        static EMPTY: &[[f32; 4]] = &[];
        self.views
            .get(id)
            .map(|v| v.selection_rects.as_slice())
            .unwrap_or(EMPTY)
    }

    fn take_anchor_click(&mut self, id: ViewId) -> Option<String> {
        let view = self.views.get_mut(id)?;
        view.container_mut().inner_mut().take_anchor_click()
    }

    fn take_pending_images(&mut self) -> Vec<(ViewId, String, String, bool)> {
        let mut result = Vec::new();
        for (id, view) in self.views.iter_mut() {
            for (src, redraw_on_ready) in view.container_mut().inner_mut().take_pending_images() {
                let baseurl = view
                    .container()
                    .image_baseurls
                    .borrow()
                    .get(&src)
                    .cloned()
                    .unwrap_or_default();
                result.push((id, src, baseurl, redraw_on_ready));
            }
        }
        result
    }

    fn load_image_from_bytes(
        &mut self,
        id: ViewId,
        url: &str,
        bytes: &[u8],
        redraw_on_ready: bool,
    ) {
        let Some(view) = self.views.get_mut(id) else {
            return;
        };
        if let Some(existing) = view.staged_images.iter_mut().find(|(u, _, _)| u == url) {
            existing.1 = bytes.to_vec();
            existing.2 = redraw_on_ready;
        } else {
            view.staged_images
                .push((url.to_string(), bytes.to_vec(), redraw_on_ready));
        }
    }

    fn set_css_cache(&mut self, id: ViewId, cache: HashMap<String, String>) {
        let Some(view) = self.views.get_mut(id) else {
            return;
        };
        view.container().set_css_cache(cache);
    }

    fn scroll_to_fragment(&mut self, id: ViewId, fragment: &str) -> bool {
        let Some(view) = self.views.get_mut(id) else {
            return false;
        };
        let state = match view.doc_state.as_ref() {
            Some(s) => s,
            None => return false,
        };
        let root = match state.doc.root() {
            Some(r) => r,
            None => return false,
        };

        // Try #id first, then [name="fragment"] (matches browser behavior).
        // Escape CSS meta-characters so fragments like "foo.bar" don't get
        // misinterpreted as compound selectors.
        let escaped = css_escape_ident(fragment);
        let id_selector = format!("#{escaped}");
        let el = root.select_one(&id_selector).or_else(|| {
            let quoted = fragment.replace('\\', "\\\\").replace('"', "\\\"");
            let name_selector = format!("[name=\"{quoted}\"]");
            root.select_one(&name_selector)
        });

        if let Some(el) = el {
            let pos = el.placement();
            let max_scroll = (view.content_height - view.size.height as f32).max(0.0);
            view.scroll_y = pos.y.clamp(0.0, max_scroll);
            true
        } else {
            false
        }
    }
}
