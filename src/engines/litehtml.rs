use iced::keyboard;
use iced::mouse::{self, Interaction};
use iced::{Point, Size};
use rand::Rng;

use super::{Engine, PageType, PixelFormat, ViewId};
use crate::ImageInfo;

use litehtml::pixbuf::PixbufContainer;
use litehtml::selection::Selection;
use litehtml::{Document, Position};

/// Persistent document and selection state for a view.
///
/// # Safety
///
/// The `doc` field borrows from the `Box<PixbufContainer>` in the parent
/// `LitehtmlView`. The container is heap-allocated for address stability.
/// `doc_state` is always dropped before the container is modified or dropped
/// (field drop order: `doc_state` is declared before `container`).
struct DocumentState {
    doc: Document<'static>,
    measure: Box<dyn Fn(&str, usize) -> f32>,
    selection: Selection<'static>,
}

struct LitehtmlView {
    id: ViewId,
    // IMPORTANT: doc_state must be declared before container so it drops first.
    doc_state: Option<DocumentState>,
    container: Box<PixbufContainer>,
    html: String,
    url: String,
    title: String,
    cursor: Interaction,
    last_frame: ImageInfo,
    needs_render: bool,
    /// Selection highlight rects in logical coords, scroll-adjusted.
    /// Drawn as iced quads by the widget so the base image Handle stays stable.
    selection_rects: Vec<[f32; 4]>,
    scroll_y: f32,
    content_height: f32,
    size: Size<u32>,
    drag_origin: Option<(f32, f32)>,
    drag_active: bool,
}

/// CPU-based HTML rendering engine backed by litehtml.
///
/// No URL navigation, no keyboard input, no JavaScript.
/// Uses `litehtml::pixbuf::PixbufContainer` for software rasterization.
pub struct Litehtml {
    views: Vec<LitehtmlView>,
    scale_factor: f32,
}

impl Default for Litehtml {
    fn default() -> Self {
        Self {
            views: Vec::new(),
            scale_factor: 1.0,
        }
    }
}

impl Litehtml {
    fn find_view(&self, id: ViewId) -> &LitehtmlView {
        self.views
            .iter()
            .find(|v| v.id == id)
            .expect("The requested View id was not found")
    }

    fn find_view_mut(&mut self, id: ViewId) -> &mut LitehtmlView {
        self.views
            .iter_mut()
            .find(|v| v.id == id)
            .expect("The requested View id was not found")
    }
}

/// Build a persistent Document for the view, storing it alongside its
/// text-measurement closure and a fresh Selection.
///
/// Drops any existing document state first (releasing the container borrow),
/// then resizes the container, creates a new Document, and renders the layout.
fn rebuild_document(view: &mut LitehtmlView) {
    view.doc_state = None;

    let w = view.size.width;
    let h = view.size.height;

    if w == 0 || h == 0 || view.html.is_empty() {
        return;
    }

    // Pass 1: use a tall viewport so CSS `100vh` doesn't cap content height.
    let layout_h = h.max(10_000);
    view.container.resize(w, layout_h);

    // Capture the text measurement closure before borrowing the container
    let measure = view.container.text_measure_fn();

    // SAFETY: Manual lifetime extension is required here due to litehtml API constraints.
    //
    // The litehtml Document<'a> type is invariant over its lifetime parameter and
    // requires a mutable borrow of the container. This makes it incompatible with
    // self-referential struct crates like ouroboros or self_cell, which cannot handle:
    //   1. Lifetime invariance (they require covariance)
    //   2. Multiple mutable borrows from the same field (Document and Selection)
    //
    // The unsafe lifetime extension to 'static is safe because:
    //   1. container is Box<PixbufContainer> — heap-allocated with a stable address
    //   2. doc_state is declared before container in LitehtmlView → drops first
    //   3. doc_state is set to None before any container modification or drop
    //   4. The Document never outlives the container it borrows from
    //
    // This pattern has been carefully reviewed and is the standard approach for
    // self-referential structures when safe abstractions are incompatible.
    let container_ptr = &mut *view.container as *mut PixbufContainer;
    let container_ref: &'static mut PixbufContainer = unsafe { &mut *container_ptr };

    match Document::from_html(&view.html, container_ref, None, None) {
        Err(e) => {
            eprintln!("litehtml: from_html failed: {e:?}");
        }
        Ok(mut doc) => {
            let _ = doc.render(w as f32);
            let measured = doc.height();

            // Pass 2: if content overflows the layout viewport, re-layout so
            // `100vh` covers the full content and overflow clips don't cut it off.
            if measured > layout_h as f32 {
                let final_h = measured.ceil() as u32;

                // Drop the document BEFORE resizing. Calling resize while doc
                // holds a &mut borrow of the container would create two live
                // &mut references — undefined behavior.
                drop(doc);

                view.container.resize(w, final_h);
                let measure2 = view.container.text_measure_fn();

                let container_ptr2 = &mut *view.container as *mut PixbufContainer;
                let container_ref2: &'static mut PixbufContainer =
                    unsafe { &mut *container_ptr2 };

                match Document::from_html(&view.html, container_ref2, None, None) {
                    Err(e) => {
                        eprintln!("litehtml: from_html pass 2 failed: {e:?}");
                        return;
                    }
                    Ok(mut doc2) => {
                        let _ = doc2.render(w as f32);
                        view.content_height = doc2.height();

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
}

/// Render the full document into the pixel buffer and update `last_frame`.
///
/// The buffer covers the entire content height so the widget can scroll
/// by offsetting the draw position — no re-render needed per scroll.
fn draw_view(view: &mut LitehtmlView) {
    let w = view.size.width;
    let full_h = (view.content_height.ceil() as u32).max(view.size.height);

    view.container.resize(w, full_h);
    // Disable CSS overflow clips so the full document is painted —
    // overflow: hidden/auto containers won't clip content below their box.
    view.container.set_ignore_overflow_clips(true);

    if let Some(ref mut state) = view.doc_state {
        let clip = Position {
            x: 0.0,
            y: 0.0,
            width: w as f32,
            height: full_h as f32,
        };
        state.doc.draw(0, 0.0, 0.0, Some(clip));
    }
    view.container.set_ignore_overflow_clips(false);

    let phys_w = view.container.width();
    let phys_h = view.container.height();
    let pixels = unpremultiply_rgba(view.container.pixels());

    view.last_frame = ImageInfo::new(pixels, PixelFormat::Rgba, phys_w, phys_h);
    view.needs_render = false;
}

/// Main render entry point: rebuilds the document if needed, then draws.
fn render_view(view: &mut LitehtmlView) {
    let w = view.size.width;
    let h = view.size.height;

    if w == 0 || h == 0 {
        return;
    }

    if view.html.is_empty() {
        let phys_w = view.container.width();
        let phys_h = view.container.height();
        view.last_frame = ImageInfo::blank(phys_w, phys_h);
        view.needs_render = false;
        return;
    }

    if view.doc_state.is_none() {
        rebuild_document(view);
    }

    draw_view(view);
}

/// Convert premultiplied-alpha RGBA pixels to straight alpha.
///
/// litehtml's pixbuf backend (tiny-skia) stores premultiplied RGBA, but
/// iced's `image::Handle::from_rgba` expects straight (unpremultiplied) alpha.
fn unpremultiply_rgba(pixels: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(pixels.len());
    for chunk in pixels.chunks_exact(4) {
        let a = chunk[3] as u32;
        if a == 0 {
            result.extend_from_slice(&[0, 0, 0, 0]);
        } else {
            let r = ((chunk[0] as u32 * 255 + a / 2) / a).min(255) as u8;
            let g = ((chunk[1] as u32 * 255 + a / 2) / a).min(255) as u8;
            let b = ((chunk[2] as u32 * 255 + a / 2) / a).min(255) as u8;
            result.extend_from_slice(&[r, g, b, chunk[3]]);
        }
    }
    result
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

    fn render(&mut self, _size: Size<u32>) {
        for view in &mut self.views {
            if view.needs_render {
                render_view(view);
            }
        }
    }

    fn request_render(&mut self, id: ViewId, _size: Size<u32>) {
        let view = self.find_view_mut(id);
        if view.needs_render {
            render_view(view);
        }
    }

    fn new_view(&mut self, size: Size<u32>, content: Option<PageType>) -> ViewId {
        let id = rand::thread_rng().gen();
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

        let mut view = LitehtmlView {
            id,
            doc_state: None,
            container: Box::new(PixbufContainer::new_with_scale(w, h, self.scale_factor)),
            html,
            url,
            title: String::new(),
            cursor: Interaction::Idle,
            last_frame: ImageInfo::blank(w, h),
            needs_render: true,
            selection_rects: Vec::new(),
            scroll_y: 0.0,
            content_height: 0.0,
            size,
            drag_origin: None,
            drag_active: false,
        };

        render_view(&mut view);
        self.views.push(view);
        id
    }

    fn remove_view(&mut self, id: ViewId) {
        self.views.retain(|v| v.id != id);
    }

    fn has_view(&self, id: ViewId) -> bool {
        self.views.iter().any(|v| v.id == id)
    }

    fn focus(&mut self) {
        // No-op: litehtml has no focus model.
    }

    fn unfocus(&self) {
        // No-op: litehtml has no focus model.
    }

    fn resize(&mut self, size: Size<u32>) {
        for view in &mut self.views {
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
        for view in &mut self.views {
            view.doc_state = None;

            view.container
                .resize_with_scale(view.size.width, view.size.height, scale);
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
                let view = self.find_view_mut(id);
                view.drag_origin = Some((point.x, point.y));
                view.drag_active = false;
                if let Some(ref mut state) = view.doc_state {
                    state.selection.clear();
                }
                view.selection_rects.clear();
            }
            mouse::Event::CursorMoved { .. } => {
                let view = self.find_view_mut(id);
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
                let view = self.find_view_mut(id);
                view.drag_active = false;
                view.drag_origin = None;
            }
            _ => {}
        }
    }

    fn scroll(&mut self, id: ViewId, delta: mouse::ScrollDelta) {
        let view = self.find_view_mut(id);
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
        let view = self.find_view_mut(id);
        match page_type {
            PageType::Html(html) => {
                view.doc_state = None;

                view.html = html;
                view.scroll_y = 0.0;
                view.needs_render = true;
            }
            PageType::Url(url) => {
                view.url = url;
            }
        }
    }

    fn refresh(&mut self, id: ViewId) {
        let view = self.find_view_mut(id);
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
        let url = &self.find_view(id).url;
        if url.is_empty() {
            "about:blank".to_string()
        } else {
            url.clone()
        }
    }

    fn get_title(&self, id: ViewId) -> String {
        self.find_view(id).title.clone()
    }

    fn get_cursor(&self, id: ViewId) -> Interaction {
        self.find_view(id).cursor
    }

    fn get_view(&self, id: ViewId) -> &ImageInfo {
        &self.find_view(id).last_frame
    }

    fn get_scroll_y(&self, id: ViewId) -> f32 {
        self.find_view(id).scroll_y
    }

    fn get_content_height(&self, id: ViewId) -> f32 {
        self.find_view(id).content_height
    }

    fn get_selected_text(&self, id: ViewId) -> Option<String> {
        self.find_view(id)
            .doc_state
            .as_ref()?
            .selection
            .selected_text()
    }

    fn get_selection_rects(&self, id: ViewId) -> &[[f32; 4]] {
        &self.find_view(id).selection_rects
    }
}
