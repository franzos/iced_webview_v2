use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use iced::keyboard;
use iced::mouse::{self, Interaction};
use iced::{Point, Size};

use super::{Engine, PageType, ViewId, ViewManager};
use crate::ImageInfo;

use anyrender::ImageRenderer;
use anyrender_vello::VelloImageRenderer;
use blitz_dom::{Document, DocumentConfig};
use blitz_html::HtmlDocument;
use blitz_net::Provider;
use blitz_paint::paint_scene;
use blitz_traits::events::{
    BlitzKeyEvent, BlitzPointerEvent, BlitzPointerId, BlitzWheelDelta, BlitzWheelEvent, KeyState,
    MouseEventButton, MouseEventButtons, PointerCoords, PointerDetails, UiEvent,
};
use blitz_traits::navigation::{NavigationOptions, NavigationProvider};
use blitz_traits::net::NetProvider;
use blitz_traits::shell::{ColorScheme, ShellProvider, Viewport};
use cursor_icon::CursorIcon;
use keyboard_types::Modifiers;
use smol_str::SmolStr;

/// Captures link clicks from the Blitz document.
struct LinkCapture(Arc<Mutex<Option<String>>>);

impl NavigationProvider for LinkCapture {
    fn navigate_to(&self, options: NavigationOptions) {
        *self.0.lock().unwrap() = Some(options.url.to_string());
    }
}

/// Shell provider that tracks cursor changes and redraw requests from Blitz.
struct WebviewShell {
    cursor: Arc<Mutex<CursorIcon>>,
    redraw_requested: Arc<AtomicBool>,
}

impl ShellProvider for WebviewShell {
    fn set_cursor(&self, icon: CursorIcon) {
        *self.cursor.lock().unwrap() = icon;
    }

    fn request_redraw(&self) {
        self.redraw_requested.store(true, Ordering::Release);
    }
}

struct BlitzView {
    document: Option<HtmlDocument>,
    net_provider: Arc<dyn NetProvider>,
    nav_capture: Arc<Mutex<Option<String>>>,
    cursor_icon: Arc<Mutex<CursorIcon>>,
    redraw_requested: Arc<AtomicBool>,
    url: String,
    title: String,
    cursor: Interaction,
    last_frame: ImageInfo,
    needs_render: bool,
    /// Tracked across CursorMoved events so PointerMove dispatches carry
    /// accurate button state — Blitz uses it to detect drag-selection.
    mouse_buttons: MouseEventButtons,
    /// Number of update ticks to keep draining resources after goto().
    /// blitz_net fetches sub-resources (images, CSS) asynchronously; we need
    /// to call resolve() periodically to pick them up. Once the budget runs
    /// out we stop polling (resolve is expensive for large documents).
    resource_ticks: u32,
    size: Size<u32>,
    scale: f32,
}

/// HTML rendering engine backed by Blitz (Stylo + Taffy + Vello).
///
/// Supports modern CSS (flexbox, grid, Firefox CSS engine via Stylo),
/// but no JavaScript. Rasterizes on the GPU via `anyrender_vello` and
/// displays through iced's shader widget.
pub struct Blitz {
    views: ViewManager<BlitzView>,
    scale_factor: f32,
    color_scheme: ColorScheme,
    gpu: GpuRasterizer,
}

fn detect_color_scheme() -> ColorScheme {
    if let Ok(val) = std::env::var("ICED_WEBVIEW_COLOR_SCHEME") {
        return match val.to_lowercase().as_str() {
            "dark" => ColorScheme::Dark,
            _ => ColorScheme::Light,
        };
    }
    if let Ok(theme) = std::env::var("GTK_THEME") {
        if theme.to_lowercase().contains("dark") {
            return ColorScheme::Dark;
        }
    }
    ColorScheme::Light
}

impl Default for Blitz {
    fn default() -> Self {
        Self {
            views: ViewManager::default(),
            scale_factor: 1.0,
            color_scheme: detect_color_scheme(),
            gpu: GpuRasterizer::new(),
        }
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
#[allow(clippy::too_many_arguments)]
fn create_document(
    html: &str,
    base_url: &str,
    net: &Arc<dyn NetProvider>,
    nav: &Arc<LinkCapture>,
    shell: &Arc<WebviewShell>,
    size: Size<u32>,
    scale: f32,
    color_scheme: ColorScheme,
) -> HtmlDocument {
    let phys_w = (size.width as f32 * scale).round() as u32;
    let phys_h = (size.height as f32 * scale).round() as u32;

    let config = DocumentConfig {
        base_url: if base_url.is_empty() {
            None
        } else {
            Some(base_url.to_string())
        },
        net_provider: Some(Arc::clone(net)),
        navigation_provider: Some(Arc::clone(nav) as Arc<dyn NavigationProvider>),
        shell_provider: Some(Arc::clone(shell) as Arc<dyn ShellProvider>),
        viewport: Some(Viewport::new(phys_w, phys_h, scale, color_scheme)),
        ..Default::default()
    };

    let mut doc = HtmlDocument::from_html(html, config);
    doc.resolve(0.0);
    doc
}

/// Persistent GPU rasterizer shared across all views.
///
/// Building a `VelloImageRenderer` triggers full wgpu init plus Vello
/// pipeline compilation (hundreds of ms), so we keep one alive and resize
/// it on demand instead of constructing per-frame.
struct GpuRasterizer {
    renderer: Option<VelloImageRenderer>,
    size: (u32, u32),
    buffer: Vec<u8>,
}

impl GpuRasterizer {
    fn new() -> Self {
        Self {
            renderer: None,
            size: (0, 0),
            buffer: Vec::new(),
        }
    }
}

/// Render the visible viewport to an RGBA pixel buffer.
///
/// Only the viewport-sized region is rasterized — `paint_scene` reads
/// `doc.viewport_scroll()` and offsets content accordingly, so scrolling
/// is owned by Blitz and the texture stays bounded by window size.
fn render_view(view: &mut BlitzView, gpu: &mut GpuRasterizer) {
    let w = view.size.width;
    let h = view.size.height;

    if w == 0 || h == 0 {
        return;
    }

    // Apply pending style/layout invalidation (e.g. :hover changes from
    // recent pointer events) before painting. Without this, hover styles
    // wouldn't appear until the next periodic resource resolve tick.
    if let Some(ref mut doc) = view.document {
        doc.resolve(0.0);
    }

    let doc = match view.document.as_ref() {
        Some(d) => d,
        None => {
            view.last_frame = ImageInfo::blank(w, h);
            view.needs_render = false;
            return;
        }
    };

    let scale = view.scale as f64;
    let render_w = (w as f64 * scale).round() as u32;
    let render_h = (h as f64 * scale).round() as u32;

    if render_w == 0 || render_h == 0 {
        view.last_frame = ImageInfo::blank(w, h);
        view.needs_render = false;
        return;
    }

    let renderer = gpu
        .renderer
        .get_or_insert_with(|| VelloImageRenderer::new(render_w, render_h));

    if gpu.size != (render_w, render_h) {
        renderer.resize(render_w, render_h);
        gpu.size = (render_w, render_h);
    }

    let expected = (render_w as usize) * (render_h as usize) * 4;
    gpu.buffer.resize(expected, 0);

    renderer.render(
        |scene| {
            paint_scene(scene, doc, scale, render_w, render_h, 0, 0);
        },
        &mut gpu.buffer,
    );

    // Hand the buffer to ImageInfo by move; next frame re-allocates via
    // `resize`. Avoids a viewport-sized clone here, and from_shader_pixels
    // avoids the parallel image::Handle clone.
    let pixels = mem::take(&mut gpu.buffer);
    view.last_frame = ImageInfo::from_shader_pixels(pixels, render_w, render_h);
    view.needs_render = false;
}

/// How many update ticks to keep draining resources after goto().
/// At 10ms per tick this gives ~30s for sub-resources to arrive.
const RESOURCE_TICK_BUDGET: u32 = 3000;

/// How often (in ticks) to actually call resolve() during the drain phase.
/// resolve() is expensive (full Stylo + Taffy layout pass), so we throttle it.
/// At ~10ms per tick, 100 ticks ≈ 1 second between resolve calls.
const RESOLVE_INTERVAL: u32 = 100;

/// Drain completed resource fetches and re-resolve the document.
fn drain_and_resolve(view: &mut BlitzView) {
    if let Some(ref mut doc) = view.document {
        doc.resolve(0.0);
    }
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
        for view in self.views.values_mut() {
            // Pick up Blitz's internal request_redraw signal (scroll, hover,
            // resource arrival, IME, etc.) and convert it to a render request.
            if view.redraw_requested.swap(false, Ordering::AcqRel) {
                view.needs_render = true;
            }
            if view.resource_ticks > 0 {
                view.resource_ticks -= 1;
                if view.resource_ticks % RESOLVE_INTERVAL == 0 {
                    drain_and_resolve(view);
                    view.needs_render = true;
                }
            }
        }
    }

    fn render(&mut self, _size: Size<u32>) {
        for view in self.views.values_mut() {
            if view.needs_render {
                render_view(view, &mut self.gpu);
            }
        }
    }

    fn request_render(&mut self, id: ViewId, _size: Size<u32>) {
        let Some(view) = self.views.get_mut(id) else {
            return;
        };
        if view.needs_render {
            render_view(view, &mut self.gpu);
        }
    }

    fn new_view(&mut self, size: Size<u32>, content: Option<PageType>) -> ViewId {
        let w = size.width.max(1);
        let h = size.height.max(1);
        let size = Size::new(w, h);

        let nav_capture = Arc::new(Mutex::new(None));
        let cursor_icon = Arc::new(Mutex::new(CursorIcon::Default));
        let redraw_requested = Arc::new(AtomicBool::new(false));
        let net = new_net_provider();
        let nav = Arc::new(LinkCapture(Arc::clone(&nav_capture)));
        let shell = Arc::new(WebviewShell {
            cursor: Arc::clone(&cursor_icon),
            redraw_requested: Arc::clone(&redraw_requested),
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
                self.color_scheme,
            ))
        } else {
            None
        };
        let has_document = document.is_some();

        let mut view = BlitzView {
            document,
            net_provider: net,
            nav_capture,
            cursor_icon,
            redraw_requested,
            url,
            title: String::new(),
            cursor: Interaction::Idle,
            last_frame: ImageInfo::blank(w, h),
            needs_render: true,
            mouse_buttons: MouseEventButtons::None,
            resource_ticks: if has_document {
                RESOURCE_TICK_BUDGET
            } else {
                0
            },
            size,
            scale: self.scale_factor,
        };

        render_view(&mut view, &mut self.gpu);
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

    fn focus(&mut self) {}

    fn unfocus(&self) {}

    fn resize(&mut self, size: Size<u32>) {
        for view in self.views.values_mut() {
            view.size = size;
            if let Some(ref mut doc) = view.document {
                let scale = view.scale;
                let phys_w = (size.width as f32 * scale).round() as u32;
                let phys_h = (size.height as f32 * scale).round() as u32;
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
        for view in self.views.values_mut() {
            view.scale = scale;
            if let Some(ref mut doc) = view.document {
                let phys_w = (view.size.width as f32 * scale).round() as u32;
                let phys_h = (view.size.height as f32 * scale).round() as u32;
                let mut vp = doc.viewport_mut();
                vp.window_size = (phys_w, phys_h);
                vp.set_hidpi_scale(scale);
                drop(vp);
                doc.resolve(0.0);
            }
            view.needs_render = true;
        }
    }

    fn handle_keyboard_event(&mut self, id: ViewId, event: keyboard::Event) {
        let Some(view) = self.views.get_mut(id) else {
            return;
        };
        if let Some(ref mut doc) = view.document {
            if let Some(ke) = iced_keyboard_to_blitz(event) {
                let ui_event = if ke.state == KeyState::Pressed {
                    UiEvent::KeyDown(ke)
                } else {
                    UiEvent::KeyUp(ke)
                };
                doc.handle_ui_event(ui_event);
            }
        }
    }

    fn handle_mouse_event(&mut self, id: ViewId, point: Point, event: mouse::Event) {
        match event {
            mouse::Event::WheelScrolled { delta } => {
                self.scroll(id, delta);
            }
            mouse::Event::ButtonPressed(btn) => {
                let (button, mask) = match btn {
                    mouse::Button::Left => (MouseEventButton::Main, MouseEventButtons::Primary),
                    mouse::Button::Right => {
                        (MouseEventButton::Secondary, MouseEventButtons::Secondary)
                    }
                    mouse::Button::Middle => {
                        (MouseEventButton::Auxiliary, MouseEventButtons::Auxiliary)
                    }
                    mouse::Button::Back => (MouseEventButton::Fourth, MouseEventButtons::Fourth),
                    mouse::Button::Forward => (MouseEventButton::Fifth, MouseEventButtons::Fifth),
                    _ => return,
                };
                let Some(view) = self.views.get_mut(id) else {
                    return;
                };
                view.mouse_buttons |= mask;
                let buttons = view.mouse_buttons;
                if let Some(ref mut doc) = view.document {
                    let doc_y = point.y + doc.viewport_scroll().y as f32;
                    doc.handle_ui_event(UiEvent::PointerDown(BlitzPointerEvent {
                        id: BlitzPointerId::Mouse,
                        is_primary: true,
                        coords: PointerCoords {
                            page_x: point.x,
                            page_y: doc_y,
                            screen_x: point.x,
                            screen_y: point.y,
                            client_x: point.x,
                            client_y: point.y,
                        },
                        button,
                        buttons,
                        mods: Modifiers::empty(),
                        details: PointerDetails::default(),
                    }));
                }
            }
            mouse::Event::CursorMoved { .. } => {
                let Some(view) = self.views.get_mut(id) else {
                    return;
                };
                let buttons = view.mouse_buttons;
                if let Some(ref mut doc) = view.document {
                    let doc_y = point.y + doc.viewport_scroll().y as f32;
                    // Dispatch as PointerMove (not direct set_hover_to) so
                    // Blitz handles drag-selection logic when a button is held.
                    doc.handle_ui_event(UiEvent::PointerMove(BlitzPointerEvent {
                        id: BlitzPointerId::Mouse,
                        is_primary: true,
                        coords: PointerCoords {
                            page_x: point.x,
                            page_y: doc_y,
                            screen_x: point.x,
                            screen_y: point.y,
                            client_x: point.x,
                            client_y: point.y,
                        },
                        button: MouseEventButton::Main,
                        buttons,
                        mods: Modifiers::empty(),
                        details: PointerDetails::default(),
                    }));
                }
                let doc_cursor = view.document.as_ref().and_then(|d| d.get_cursor());
                let shell_cursor = *view.cursor_icon.lock().unwrap();
                let icon = doc_cursor.unwrap_or(shell_cursor);
                view.cursor = cursor_icon_to_interaction(icon);
            }
            mouse::Event::ButtonReleased(btn) => {
                let (button, mask) = match btn {
                    mouse::Button::Left => (MouseEventButton::Main, MouseEventButtons::Primary),
                    mouse::Button::Right => {
                        (MouseEventButton::Secondary, MouseEventButtons::Secondary)
                    }
                    mouse::Button::Middle => {
                        (MouseEventButton::Auxiliary, MouseEventButtons::Auxiliary)
                    }
                    mouse::Button::Back => (MouseEventButton::Fourth, MouseEventButtons::Fourth),
                    mouse::Button::Forward => (MouseEventButton::Fifth, MouseEventButtons::Fifth),
                    _ => return,
                };
                let Some(view) = self.views.get_mut(id) else {
                    return;
                };
                view.mouse_buttons.remove(mask);
                let buttons = view.mouse_buttons;
                if let Some(ref mut doc) = view.document {
                    let doc_y = point.y + doc.viewport_scroll().y as f32;
                    doc.handle_ui_event(UiEvent::PointerUp(BlitzPointerEvent {
                        id: BlitzPointerId::Mouse,
                        is_primary: true,
                        coords: PointerCoords {
                            page_x: point.x,
                            page_y: doc_y,
                            screen_x: point.x,
                            screen_y: point.y,
                            client_x: point.x,
                            client_y: point.y,
                        },
                        button,
                        buttons,
                        mods: Modifiers::empty(),
                        details: PointerDetails::default(),
                    }));
                }
            }
            mouse::Event::CursorLeft => {
                if let Some(view) = self.views.get_mut(id) {
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
        let Some(ref mut doc) = view.document else {
            return;
        };

        // iced and Blitz agree on sign: positive y = scroll up. Blitz
        // clamps to the document bounds and calls request_redraw internally.
        let delta = match delta {
            mouse::ScrollDelta::Lines { x, y } => {
                BlitzWheelDelta::Pixels(x as f64 * 40.0, y as f64 * 40.0)
            }
            mouse::ScrollDelta::Pixels { x, y } => BlitzWheelDelta::Pixels(x as f64, y as f64),
        };

        doc.handle_ui_event(UiEvent::Wheel(BlitzWheelEvent {
            delta,
            coords: PointerCoords {
                page_x: 0.0,
                page_y: 0.0,
                screen_x: 0.0,
                screen_y: 0.0,
                client_x: 0.0,
                client_y: 0.0,
            },
            buttons: MouseEventButtons::None,
            mods: Modifiers::empty(),
        }));
    }

    fn goto(&mut self, id: ViewId, page_type: PageType) {
        let color_scheme = self.color_scheme;
        let Some(view) = self.views.get_mut(id) else {
            return;
        };
        match page_type {
            PageType::Html(html) => {
                let nav = Arc::new(LinkCapture(Arc::clone(&view.nav_capture)));
                let shell = Arc::new(WebviewShell {
                    cursor: Arc::clone(&view.cursor_icon),
                    redraw_requested: Arc::clone(&view.redraw_requested),
                });
                let net = new_net_provider();
                view.net_provider = Arc::clone(&net);

                view.document = Some(create_document(
                    &html,
                    &view.url,
                    &net,
                    &nav,
                    &shell,
                    view.size,
                    view.scale,
                    color_scheme,
                ));
                view.needs_render = true;
                view.resource_ticks = RESOURCE_TICK_BUDGET;
            }
            PageType::Url(url) => {
                view.url = url;
            }
        }
    }

    fn refresh(&mut self, id: ViewId) {
        let Some(view) = self.views.get_mut(id) else {
            return;
        };
        if let Some(ref mut doc) = view.document {
            doc.resolve(0.0);
        }
        view.needs_render = true;
    }

    fn go_forward(&mut self, _id: ViewId) {}

    fn go_back(&mut self, _id: ViewId) {}

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
        self.views
            .get(id)
            .and_then(|v| v.document.as_ref())
            .map(|d| d.viewport_scroll().y as f32)
            .unwrap_or(0.0)
    }

    /// Returning 0 tells the widget layer the engine manages scrolling
    /// itself, which routes the view through the shader widget.
    fn get_content_height(&self, _id: ViewId) -> f32 {
        0.0
    }

    fn scroll_to_fragment(&mut self, id: ViewId, fragment: &str) -> bool {
        let Some(view) = self.views.get_mut(id) else {
            return false;
        };
        let doc = match view.document.as_ref() {
            Some(d) => d,
            None => return false,
        };

        // Try #id first (fast HashMap lookup), then [name="fragment"] via CSS selector.
        let node_id = doc.get_element_by_id(fragment).or_else(|| {
            let quoted = fragment.replace('\\', "\\\\").replace('"', "\\\"");
            doc.query_selector(&format!("[name=\"{quoted}\"]"))
                .ok()
                .flatten()
        });

        if let Some(nid) = node_id {
            if let Some(node) = doc.get_node(nid) {
                let pos = node.absolute_position(0.0, 0.0);
                let target_y = pos.y.max(0.0) as f64;
                if let Some(ref mut doc) = view.document {
                    doc.set_viewport_scroll(blitz_dom::Point {
                        x: 0.0,
                        y: target_y,
                    });
                }
                view.needs_render = true;
                return true;
            }
        }

        false
    }

    fn take_anchor_click(&mut self, id: ViewId) -> Option<String> {
        self.views.get_mut(id)?.nav_capture.lock().unwrap().take()
    }
}

fn iced_keyboard_to_blitz(event: keyboard::Event) -> Option<BlitzKeyEvent> {
    use keyboard_types::{Code, Key, Location};

    let (state, iced_key, iced_mods) = match event {
        keyboard::Event::KeyPressed { key, modifiers, .. } => (KeyState::Pressed, key, modifiers),
        keyboard::Event::KeyReleased { key, modifiers, .. } => (KeyState::Released, key, modifiers),
        _ => return None,
    };

    let kt_key = iced_key_to_blitz_key(&iced_key)?;

    let mut mods = Modifiers::empty();
    if iced_mods.shift() {
        mods |= Modifiers::SHIFT;
    }
    if iced_mods.control() {
        mods |= Modifiers::CONTROL;
    }
    if iced_mods.alt() {
        mods |= Modifiers::ALT;
    }
    if iced_mods.logo() {
        mods |= Modifiers::META;
    }

    let text = if state == KeyState::Pressed {
        match &kt_key {
            Key::Character(s) => Some(SmolStr::new(s)),
            _ => None,
        }
    } else {
        None
    };

    Some(BlitzKeyEvent {
        key: kt_key,
        code: Code::Unidentified,
        modifiers: mods,
        location: Location::Standard,
        is_auto_repeating: false,
        is_composing: false,
        state,
        text,
    })
}

fn iced_key_to_blitz_key(key: &keyboard::Key) -> Option<keyboard_types::Key> {
    use keyboard::key::Named;

    match key {
        keyboard::Key::Character(s) => Some(keyboard_types::Key::Character(s.to_string())),
        keyboard::Key::Named(named) => {
            let k = match named {
                Named::Enter => keyboard_types::Key::Enter,
                Named::Tab => keyboard_types::Key::Tab,
                Named::Space => keyboard_types::Key::Character(" ".to_string()),
                Named::Backspace => keyboard_types::Key::Backspace,
                Named::Delete => keyboard_types::Key::Delete,
                Named::Escape => keyboard_types::Key::Escape,
                Named::Insert => keyboard_types::Key::Insert,
                Named::CapsLock => keyboard_types::Key::CapsLock,
                Named::NumLock => keyboard_types::Key::NumLock,
                Named::ScrollLock => keyboard_types::Key::ScrollLock,
                Named::Pause => keyboard_types::Key::Pause,
                Named::PrintScreen => keyboard_types::Key::PrintScreen,
                Named::ContextMenu => keyboard_types::Key::ContextMenu,
                Named::ArrowDown => keyboard_types::Key::ArrowDown,
                Named::ArrowLeft => keyboard_types::Key::ArrowLeft,
                Named::ArrowRight => keyboard_types::Key::ArrowRight,
                Named::ArrowUp => keyboard_types::Key::ArrowUp,
                Named::End => keyboard_types::Key::End,
                Named::Home => keyboard_types::Key::Home,
                Named::PageDown => keyboard_types::Key::PageDown,
                Named::PageUp => keyboard_types::Key::PageUp,
                Named::F1 => keyboard_types::Key::F1,
                Named::F2 => keyboard_types::Key::F2,
                Named::F3 => keyboard_types::Key::F3,
                Named::F4 => keyboard_types::Key::F4,
                Named::F5 => keyboard_types::Key::F5,
                Named::F6 => keyboard_types::Key::F6,
                Named::F7 => keyboard_types::Key::F7,
                Named::F8 => keyboard_types::Key::F8,
                Named::F9 => keyboard_types::Key::F9,
                Named::F10 => keyboard_types::Key::F10,
                Named::F11 => keyboard_types::Key::F11,
                Named::F12 => keyboard_types::Key::F12,
                _ => return None,
            };
            Some(k)
        }
        _ => None,
    }
}
