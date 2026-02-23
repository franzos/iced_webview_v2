use std::sync::{Arc, Mutex};

use iced::keyboard;
use iced::mouse::{self, Interaction};
use iced::{Point, Size};
use rand::Rng;

use super::{Engine, PageType, PixelFormat, ViewId};
use crate::ImageInfo;

use anyrender::render_to_buffer;
use anyrender_vello_cpu::VelloCpuImageRenderer;
use blitz_dom::{Document, DocumentConfig};
use blitz_html::HtmlDocument;
use blitz_net::Provider;
use blitz_paint::paint_scene;
use blitz_traits::events::{
    BlitzPointerEvent, BlitzPointerId, MouseEventButton, MouseEventButtons, PointerCoords,
    PointerDetails, UiEvent,
};
use blitz_traits::navigation::{NavigationOptions, NavigationProvider};
use blitz_traits::net::NetProvider;
use blitz_traits::shell::{ColorScheme, ShellProvider, Viewport};
use cursor_icon::CursorIcon;
use keyboard_types::Modifiers;

/// Captures link clicks from the Blitz document.
struct LinkCapture(Arc<Mutex<Option<String>>>);

impl NavigationProvider for LinkCapture {
    fn navigate_to(&self, options: NavigationOptions) {
        *self.0.lock().unwrap() = Some(options.url.to_string());
    }
}

/// Shell provider that tracks cursor and redraw requests.
struct WebviewShell {
    cursor: Arc<Mutex<CursorIcon>>,
}

impl ShellProvider for WebviewShell {
    fn set_cursor(&self, icon: CursorIcon) {
        *self.cursor.lock().unwrap() = icon;
    }
}

struct BlitzView {
    id: ViewId,
    document: Option<HtmlDocument>,
    net_provider: Arc<dyn NetProvider>,
    nav_capture: Arc<Mutex<Option<String>>>,
    cursor_icon: Arc<Mutex<CursorIcon>>,
    url: String,
    title: String,
    cursor: Interaction,
    last_frame: ImageInfo,
    needs_render: bool,
    /// Number of update ticks to keep draining resources after goto().
    /// blitz_net fetches sub-resources (images, CSS) asynchronously; we need
    /// to call resolve() periodically to pick them up. Once the budget runs
    /// out we stop polling (resolve is expensive for large documents).
    resource_ticks: u32,
    scroll_y: f32,
    content_height: f32,
    size: Size<u32>,
    scale: f32,
}

/// CPU-based HTML rendering engine backed by Blitz (Stylo + Taffy + Vello).
///
/// Supports modern CSS (flexbox, grid, Firefox CSS engine via Stylo),
/// but no JavaScript. Uses `anyrender_vello_cpu` for software rasterization.
pub struct Blitz {
    views: Vec<BlitzView>,
    scale_factor: f32,
}

impl Default for Blitz {
    fn default() -> Self {
        Self {
            views: Vec::new(),
            scale_factor: 1.0,
        }
    }
}

impl Blitz {
    fn find_view(&self, id: ViewId) -> &BlitzView {
        self.views
            .iter()
            .find(|v| v.id == id)
            .expect("The requested View id was not found")
    }

    fn find_view_mut(&mut self, id: ViewId) -> &mut BlitzView {
        self.views
            .iter_mut()
            .find(|v| v.id == id)
            .expect("The requested View id was not found")
    }
}

fn cursor_icon_to_interaction(icon: CursorIcon) -> Interaction {
    match icon {
        CursorIcon::Pointer => Interaction::Pointer,
        CursorIcon::Text => Interaction::Text,
        CursorIcon::Crosshair => Interaction::Crosshair,
        CursorIcon::Grab => Interaction::Grab,
        CursorIcon::Grabbing => Interaction::Grabbing,
        CursorIcon::NotAllowed | CursorIcon::NoDrop => Interaction::NotAllowed,
        CursorIcon::ColResize | CursorIcon::EwResize => Interaction::ResizingHorizontally,
        CursorIcon::RowResize | CursorIcon::NsResize => Interaction::ResizingVertically,
        CursorIcon::ZoomIn => Interaction::ZoomIn,
        CursorIcon::ZoomOut => Interaction::ZoomOut,
        CursorIcon::Wait | CursorIcon::Progress => Interaction::Idle,
        _ => Interaction::Idle,
    }
}

/// Create a new net provider for sub-resource fetching.
fn new_net_provider() -> Arc<dyn NetProvider> {
    Provider::shared(None)
}

/// Parse HTML into a Blitz document with the given configuration.
fn create_document(
    html: &str,
    base_url: &str,
    net: &Arc<dyn NetProvider>,
    nav: &Arc<LinkCapture>,
    shell: &Arc<WebviewShell>,
    size: Size<u32>,
    scale: f32,
) -> HtmlDocument {
    let phys_w = (size.width as f32 * scale) as u32;
    let phys_h = (size.height as f32 * scale) as u32;

    let config = DocumentConfig {
        base_url: if base_url.is_empty() {
            None
        } else {
            Some(base_url.to_string())
        },
        net_provider: Some(Arc::clone(net)),
        navigation_provider: Some(Arc::clone(nav) as Arc<dyn NavigationProvider>),
        shell_provider: Some(Arc::clone(shell) as Arc<dyn ShellProvider>),
        viewport: Some(Viewport::new(phys_w, phys_h, scale, ColorScheme::Light)),
        ..Default::default()
    };

    let mut doc = HtmlDocument::from_html(html, config);
    doc.resolve(0.0);
    doc
}

/// Max render height in logical pixels. Prevents multi-hundred-MB pixel
/// buffers for very tall documents (e.g. docs.rs pages). Content beyond
/// this height is reachable via scrolling but not pre-rasterized.
const MAX_RENDER_HEIGHT: f32 = 8192.0;

/// Render the document to an RGBA pixel buffer.
///
/// The buffer height is capped at `MAX_RENDER_HEIGHT` logical pixels to
/// keep memory and CPU usage bounded. The widget layer uses `content_height`
/// / `scroll_y` for scroll calculations; `content_height` is clamped to the
/// rendered height so the scrollbar range matches what's actually rasterized.
fn render_view(view: &mut BlitzView) {
    let w = view.size.width;
    let h = view.size.height;

    if w == 0 || h == 0 {
        return;
    }

    let doc = match view.document.as_ref() {
        Some(d) => d,
        None => {
            view.last_frame = ImageInfo::blank(w, h);
            view.needs_render = false;
            return;
        }
    };

    let root_height = doc.root_element().final_layout.size.height;
    let capped_height = root_height.min(MAX_RENDER_HEIGHT);
    view.content_height = capped_height;

    let scale = view.scale as f64;
    let render_w = (w as f64 * scale) as u32;
    let render_h = ((capped_height as f64).max(h as f64) * scale) as u32;

    if render_w == 0 || render_h == 0 {
        view.last_frame = ImageInfo::blank(w, h);
        view.needs_render = false;
        return;
    }

    let buffer = render_to_buffer::<VelloCpuImageRenderer, _>(
        |scene| {
            paint_scene(scene, doc, scale, render_w, render_h, 0, 0);
        },
        render_w,
        render_h,
    );

    view.last_frame = ImageInfo::new(buffer, PixelFormat::Rgba, render_w, render_h);
    view.needs_render = false;
}

/// How many update ticks to keep draining resources after goto().
/// At 10ms per tick this gives ~30s for sub-resources to arrive.
const RESOURCE_TICK_BUDGET: u32 = 3000;

/// Drain completed resource fetches and re-resolve if something changed.
/// Only called while `resource_ticks > 0` (after a goto).
fn drain_and_resolve(view: &mut BlitzView) -> bool {
    let doc = match view.document.as_mut() {
        Some(d) => d,
        None => return false,
    };
    let height_before = doc.root_element().final_layout.size.height;
    doc.resolve(0.0);
    let height_after = doc.root_element().final_layout.size.height;
    height_before != height_after
}

impl Engine for Blitz {
    /// Blitz cannot fetch the initial HTML page from a URL — the widget layer
    /// handles that via `fetch_html`. However, all sub-resource fetching
    /// (images, CSS `@import`) is handled internally by `blitz_net::Provider`,
    /// so the widget layer's image pipeline (`take_pending_images`,
    /// `load_image_from_bytes`) is not used. Returning `false` here is correct
    /// for its intended purpose: telling the widget layer to fetch page HTML.
    fn handles_urls(&self) -> bool {
        false
    }

    fn update(&mut self) {
        for view in &mut self.views {
            if view.resource_ticks > 0 {
                view.resource_ticks -= 1;
                if drain_and_resolve(view) {
                    view.needs_render = true;
                }
            }
        }
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
        let size = Size::new(w, h);

        let nav_capture = Arc::new(Mutex::new(None));
        let cursor_icon = Arc::new(Mutex::new(CursorIcon::Default));
        let net = new_net_provider();
        let nav = Arc::new(LinkCapture(Arc::clone(&nav_capture)));
        let shell = Arc::new(WebviewShell {
            cursor: Arc::clone(&cursor_icon),
        });

        let (html, url) = match &content {
            Some(PageType::Html(html)) => (html.clone(), String::new()),
            Some(PageType::Url(url)) => (String::new(), url.clone()),
            None => (String::new(), String::new()),
        };

        let document = if !html.is_empty() {
            Some(create_document(
                &html,
                &url,
                &net,
                &nav,
                &shell,
                size,
                self.scale_factor,
            ))
        } else {
            None
        };
        let has_document = document.is_some();

        let mut view = BlitzView {
            id,
            document,
            net_provider: net,
            nav_capture,
            cursor_icon,
            url,
            title: String::new(),
            cursor: Interaction::Idle,
            last_frame: ImageInfo::blank(w, h),
            needs_render: true,
            resource_ticks: if has_document {
                RESOURCE_TICK_BUDGET
            } else {
                0
            },
            scroll_y: 0.0,
            content_height: 0.0,
            size,
            scale: self.scale_factor,
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

    fn view_ids(&self) -> Vec<ViewId> {
        self.views.iter().map(|v| v.id).collect()
    }

    fn focus(&mut self) {}

    fn unfocus(&self) {}

    fn resize(&mut self, size: Size<u32>) {
        for view in &mut self.views {
            view.size = size;
            if let Some(ref mut doc) = view.document {
                let scale = view.scale;
                let phys_w = (size.width as f32 * scale) as u32;
                let phys_h = (size.height as f32 * scale) as u32;
                let mut vp = doc.viewport_mut();
                vp.window_size = (phys_w, phys_h);
                drop(vp);
                doc.resolve(0.0);
            }
            view.needs_render = true;
        }
    }

    fn set_scale_factor(&mut self, scale: f32) {
        if (self.scale_factor - scale).abs() < f32::EPSILON {
            return;
        }
        self.scale_factor = scale;
        for view in &mut self.views {
            view.scale = scale;
            if let Some(ref mut doc) = view.document {
                let phys_w = (view.size.width as f32 * scale) as u32;
                let phys_h = (view.size.height as f32 * scale) as u32;
                let mut vp = doc.viewport_mut();
                vp.window_size = (phys_w, phys_h);
                vp.set_hidpi_scale(scale);
                drop(vp);
                doc.resolve(0.0);
            }
            view.needs_render = true;
        }
    }

    fn handle_keyboard_event(&mut self, _id: ViewId, _event: keyboard::Event) {
        // TODO: blitz-dom supports keyboard events (text input, Tab focus,
        // copy/paste) via UiEvent::KeyDown/KeyUp — we need to translate iced
        // keyboard::Event into BlitzKeyEvent and call
        // doc.handle_ui_event(UiEvent::KeyDown(..)) here.
    }

    fn handle_mouse_event(&mut self, id: ViewId, point: Point, event: mouse::Event) {
        match event {
            mouse::Event::WheelScrolled { delta } => {
                self.scroll(id, delta);
            }
            mouse::Event::ButtonPressed(mouse::Button::Left) => {
                let view = self.find_view_mut(id);
                if let Some(ref mut doc) = view.document {
                    let doc_y = point.y + view.scroll_y;
                    doc.handle_ui_event(UiEvent::PointerDown(BlitzPointerEvent {
                        id: BlitzPointerId::Mouse,
                        is_primary: true,
                        coords: PointerCoords {
                            page_x: point.x,
                            page_y: doc_y,
                            screen_x: point.x,
                            screen_y: doc_y,
                            client_x: point.x,
                            client_y: doc_y,
                        },
                        button: MouseEventButton::Main,
                        buttons: MouseEventButtons::Primary,
                        mods: Modifiers::empty(),
                        details: PointerDetails::default(),
                    }));
                }
            }
            mouse::Event::CursorMoved { .. } => {
                let view = self.find_view_mut(id);
                if let Some(ref mut doc) = view.document {
                    let doc_y = point.y + view.scroll_y;
                    doc.set_hover_to(point.x, doc_y);
                }
                // Update cursor icon without re-rendering — matching litehtml
                // behaviour. A full re-render for :hover CSS would be too
                // expensive with CPU rasterization.
                let doc_cursor = view.document.as_ref().and_then(|d| d.get_cursor());
                let shell_cursor = *view.cursor_icon.lock().unwrap();
                let icon = doc_cursor.unwrap_or(shell_cursor);
                view.cursor = cursor_icon_to_interaction(icon);
            }
            mouse::Event::ButtonReleased(mouse::Button::Left) => {
                let view = self.find_view_mut(id);
                if let Some(ref mut doc) = view.document {
                    let doc_y = point.y + view.scroll_y;
                    doc.handle_ui_event(UiEvent::PointerUp(BlitzPointerEvent {
                        id: BlitzPointerId::Mouse,
                        is_primary: true,
                        coords: PointerCoords {
                            page_x: point.x,
                            page_y: doc_y,
                            screen_x: point.x,
                            screen_y: doc_y,
                            client_x: point.x,
                            client_y: doc_y,
                        },
                        button: MouseEventButton::Main,
                        buttons: MouseEventButtons::None,
                        mods: Modifiers::empty(),
                        details: PointerDetails::default(),
                    }));
                }
            }
            mouse::Event::CursorLeft => {
                let view = self.find_view_mut(id);
                view.cursor = Interaction::Idle;
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
                let nav = Arc::new(LinkCapture(Arc::clone(&view.nav_capture)));
                let shell = Arc::new(WebviewShell {
                    cursor: Arc::clone(&view.cursor_icon),
                });
                let net = new_net_provider();
                view.net_provider = Arc::clone(&net);

                view.document = Some(create_document(
                    &html, &view.url, &net, &nav, &shell, view.size, view.scale,
                ));
                view.scroll_y = 0.0;
                view.needs_render = true;
                view.resource_ticks = RESOURCE_TICK_BUDGET;
            }
            PageType::Url(url) => {
                view.url = url;
            }
        }
    }

    fn refresh(&mut self, id: ViewId) {
        let view = self.find_view_mut(id);
        if let Some(ref mut doc) = view.document {
            doc.resolve(0.0);
        }
        view.needs_render = true;
    }

    fn go_forward(&mut self, _id: ViewId) {}

    fn go_back(&mut self, _id: ViewId) {}

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

    fn take_anchor_click(&mut self, id: ViewId) -> Option<String> {
        self.find_view_mut(id).nav_capture.lock().unwrap().take()
    }
}
