use iced::keyboard;
use iced::mouse::{self, Interaction};
use iced::{Point, Size};
use rand::Rng;

use super::{Engine, PageType, PixelFormat, ViewId};
use crate::ImageInfo;

use litehtml::pixbuf::PixbufContainer;
use litehtml::{Document, Position};

struct LitehtmlView {
    id: ViewId,
    container: PixbufContainer,
    html: String,
    title: String,
    cursor: Interaction,
    last_frame: ImageInfo,
    needs_render: bool,
    scroll_y: f32,
    content_height: f32,
    size: Size<u32>,
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

/// Render a view's HTML into its container, extracting pixels into last_frame.
///
/// Creates a temporary `Document`, lays it out, draws it, then drops the
/// document. This sidesteps the lifetime constraint (`Document<'a>` borrows
/// the container) by never storing the document across frames.
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

    // Clear the pixmap (logical dims — container scales internally)
    view.container.resize(w, h);

    match Document::from_html(&view.html, &mut view.container, None, None) {
        Err(e) => {
            eprintln!("litehtml: from_html failed: {e:?}");
        }
        Ok(mut doc) => {
            // CSS layout at logical width
            let _ = doc.render(w as f32);
            view.content_height = doc.height();

            let max_scroll = (view.content_height - h as f32).max(0.0);
            view.scroll_y = view.scroll_y.clamp(0.0, max_scroll);

            // Clip rect in logical coordinates
            let clip = Position {
                x: 0.0,
                y: 0.0,
                width: w as f32,
                height: h as f32,
            };
            doc.draw(0, 0.0, -view.scroll_y, Some(clip));
        }
    }

    // Pixel buffer is at physical resolution
    let phys_w = view.container.width();
    let phys_h = view.container.height();
    let pixels = unpremultiply_rgba(view.container.pixels());

    view.last_frame = ImageInfo::new(pixels, PixelFormat::Rgba, phys_w, phys_h);
    view.needs_render = false;
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

impl Engine for Litehtml {
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

        let mut view = LitehtmlView {
            id,
            container: PixbufContainer::new_with_scale(w, h, self.scale_factor),
            html,
            title: String::new(),
            cursor: Interaction::Idle,
            last_frame: ImageInfo::blank(w, h),
            needs_render: true,
            scroll_y: 0.0,
            content_height: 0.0,
            size,
        };

        render_view(&mut view);
        self.views.push(view);
        id
    }

    fn remove_view(&mut self, id: ViewId) {
        self.views.retain(|v| v.id != id);
    }

    fn focus(&mut self) {
        // No-op: litehtml has no focus model.
    }

    fn unfocus(&self) {
        // No-op: litehtml has no focus model.
    }

    fn resize(&mut self, size: Size<u32>) {
        for view in &mut self.views {
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
            view.container
                .resize_with_scale(view.size.width, view.size.height, scale);
            view.needs_render = true;
        }
    }

    fn handle_keyboard_event(&mut self, _id: ViewId, _event: keyboard::Event) {
        // No-op: litehtml has no keyboard interaction.
    }

    fn handle_mouse_event(&mut self, id: ViewId, _point: Point, event: mouse::Event) {
        match event {
            mouse::Event::WheelScrolled { delta } => self.scroll(id, delta),
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
        view.needs_render = true;
    }

    fn goto(&mut self, id: ViewId, page_type: PageType) {
        let view = self.find_view_mut(id);
        match page_type {
            PageType::Html(html) => {
                view.html = html;
                view.scroll_y = 0.0;
                view.needs_render = true;
            }
            PageType::Url(_) => {
                // No-op: litehtml renderer does not fetch remote URLs.
            }
        }
    }

    fn refresh(&mut self, id: ViewId) {
        self.find_view_mut(id).needs_render = true;
    }

    fn go_forward(&mut self, _id: ViewId) {
        // No-op: no navigation history.
    }

    fn go_back(&mut self, _id: ViewId) {
        // No-op: no navigation history.
    }

    fn get_url(&self, _id: ViewId) -> String {
        "about:blank".to_string()
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
}
