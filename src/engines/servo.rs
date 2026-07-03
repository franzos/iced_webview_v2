use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use iced::keyboard;
use iced::mouse::{self, Interaction};
use iced::{Point, Size};
use tokio::sync::Notify;

use super::{Engine, PageType, PixelFormat, ViewId, ViewManager};
use crate::ImageInfo;

use dpi::PhysicalSize;
use servo::{
    Cursor, InputEvent, KeyboardEvent, MouseButton as ServoMouseButton, MouseButtonAction,
    MouseButtonEvent, MouseMoveEvent, RenderingContext, Servo as ServoInstance, ServoBuilder,
    SoftwareRenderingContext, WebView, WebViewBuilder, WebViewDelegate, WheelDelta, WheelEvent,
    WheelMode,
};
use servo::{
    DeviceIndependentPixel, DeviceIntRect, DeviceIntSize, DevicePixel, DevicePoint, WebViewPoint,
};
use url::Url;

/// Event-driven waker that Servo calls (from any thread) whenever it wants the
/// embedder to spin its event loop. The `Notify` coalesces multiple wake signals
/// into a single pending notification, so a burst of calls produces at most one
/// wake-up on the consumer side.
#[derive(Clone)]
struct ServoWaker {
    notify: Arc<Notify>,
}

impl servo::EventLoopWaker for ServoWaker {
    fn clone_box(&self) -> Box<dyn servo::EventLoopWaker> {
        Box::new(self.clone())
    }

    fn wake(&self) {
        self.notify.notify_one();
    }
}

/// Hashable handle used to give the Servo wake subscription a stable identity
/// in the iced runtime. Hashing the `Arc`'s pointer address is enough —
/// each `Servo` instance has its own `Notify`, so two different engines get
/// two distinct subscriptions.
#[derive(Clone)]
struct WakeSubId(Arc<Notify>);

impl Hash for WakeSubId {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.0) as usize).hash(state);
    }
}

/// Shared mutable state populated by the `WebViewDelegate` callbacks and
/// drained each `update()` tick.
struct DelegateState {
    url: RefCell<Option<String>>,
    title: RefCell<Option<String>>,
    cursor: RefCell<Cursor>,
    frame_ready: RefCell<bool>,
}

/// Per-webview delegate that writes into a shared `DelegateState`.
struct ViewDelegate {
    state: Rc<DelegateState>,
}

impl WebViewDelegate for ViewDelegate {
    fn notify_url_changed(&self, _webview: WebView, url: Url) {
        *self.state.url.borrow_mut() = Some(url.to_string());
    }

    fn notify_page_title_changed(&self, _webview: WebView, title: Option<String>) {
        *self.state.title.borrow_mut() = title;
    }

    fn notify_cursor_changed(&self, _webview: WebView, cursor: Cursor) {
        *self.state.cursor.borrow_mut() = cursor;
    }

    fn notify_new_frame_ready(&self, _webview: WebView) {
        *self.state.frame_ready.borrow_mut() = true;
    }
}

struct ServoView {
    webview: WebView,
    delegate_state: Rc<DelegateState>,
    url: String,
    title: String,
    cursor: Interaction,
    last_frame: ImageInfo,
    needs_render: bool,
    size: Size<u32>,
    last_cursor: DevicePoint,
}

/// Full browser engine backed by [Servo](https://servo.org/) (HTML5, CSS3, JS).
///
/// Servo handles its own networking, scrolling, and JavaScript execution.
/// Rendering is software-based via `SoftwareRenderingContext`, producing RGBA
/// pixel buffers that map directly to iced's image widget.
///
/// ## Text selection / clipboard
///
/// Servo manages text selection and clipboard operations (Ctrl+C / Ctrl+V)
/// internally — the selected text is rendered as part of the painted frame and
/// copy/paste goes through Servo's `ClipboardDelegate`. The embedding API does
/// not expose a way to query the current DOM selection, so `get_selected_text()`
/// and `get_selection_rects()` cannot be implemented and use the default (empty)
/// trait implementations.
pub struct Servo {
    instance: ServoInstance,
    rendering_context: Rc<SoftwareRenderingContext>,
    views: ViewManager<ServoView>,
    scale_factor: f32,
    /// Shared with `ServoWaker` — Servo signals this whenever it wants the
    /// embedder to call `spin_event_loop`. The iced subscription exposed by
    /// [`Servo::subscription`] awaits the same handle.
    notify: Arc<Notify>,
}

impl Default for Servo {
    /// Panics if the rendering context cannot be created — see
    /// [`Servo::try_new`].
    fn default() -> Self {
        Self::try_new().expect("failed to initialize Servo engine")
    }
}

impl Servo {
    /// Create the Servo engine, returning an error if the software
    /// rendering context cannot be created.
    pub fn try_new() -> Result<Self, String> {
        let size = PhysicalSize::new(ImageInfo::WIDTH, ImageInfo::HEIGHT);
        let rendering_context = SoftwareRenderingContext::new(size)
            .map_err(|e| format!("failed to create SoftwareRenderingContext: {e:?}"))?;
        let rendering_context = Rc::new(rendering_context);

        let notify = Arc::new(Notify::new());
        let waker = ServoWaker {
            notify: Arc::clone(&notify),
        };

        let instance = ServoBuilder::default()
            .event_loop_waker(Box::new(waker))
            .build();

        Ok(Self {
            instance,
            rendering_context,
            views: ViewManager::default(),
            scale_factor: 1.0,
            notify,
        })
    }

    /// An iced [`Subscription`] that yields [`Action::Update`] whenever Servo
    /// signals it has work to do, plus a 500ms safety tick so the event loop
    /// still runs if a wake is somehow missed. This replaces the hardcoded
    /// `time::every(10ms)` pattern used for other engines.
    ///
    /// [`Subscription`]: iced::Subscription
    /// [`Action::Update`]: crate::Action::Update
    pub fn subscription(&self) -> iced::Subscription<crate::Action> {
        use iced::futures::SinkExt;

        let id = WakeSubId(Arc::clone(&self.notify));

        let wake_stream = iced::Subscription::run_with(id, |id| {
            let notify = Arc::clone(&id.0);
            iced::stream::channel(1, async move |mut output| loop {
                notify.notified().await;
                let _ = output.send(crate::Action::Update).await;
            })
        });

        let fallback = iced::time::every(Duration::from_millis(500)).map(|_| crate::Action::Update);

        iced::Subscription::batch([wake_stream, fallback])
    }
}

fn cursor_to_interaction(cursor: Cursor) -> Interaction {
    match cursor {
        Cursor::Pointer => Interaction::Pointer,
        Cursor::Text | Cursor::VerticalText => Interaction::Text,
        Cursor::Crosshair | Cursor::Cell => Interaction::Crosshair,
        Cursor::Grab | Cursor::AllScroll => Interaction::Grab,
        Cursor::Grabbing => Interaction::Grabbing,
        Cursor::NotAllowed | Cursor::NoDrop => Interaction::NotAllowed,
        Cursor::ColResize | Cursor::EwResize | Cursor::EResize | Cursor::WResize => {
            Interaction::ResizingHorizontally
        }
        Cursor::RowResize | Cursor::NsResize | Cursor::NResize | Cursor::SResize => {
            Interaction::ResizingVertically
        }
        Cursor::ZoomIn => Interaction::ZoomIn,
        Cursor::ZoomOut => Interaction::ZoomOut,
        _ => Interaction::Idle,
    }
}

/// Logical (device-independent) size → physical device pixels. Servo lays out
/// CSS at `device_size / hidpi`, so we feed it a physical-sized buffer and a
/// matching hidpi factor — otherwise content is scaled twice on HiDPI displays.
fn physical_size(size: Size<u32>, scale: f32) -> PhysicalSize<u32> {
    PhysicalSize::new(
        (size.width as f32 * scale).round().max(1.0) as u32,
        (size.height as f32 * scale).round().max(1.0) as u32,
    )
}

/// Paint a webview and capture the pixel buffer into `ImageInfo`.
fn capture_frame(view: &mut ServoView, rendering_context: &SoftwareRenderingContext, scale: f32) {
    let phys = physical_size(view.size, scale);
    let (w, h) = (phys.width, phys.height);
    if w == 0 || h == 0 {
        return;
    }

    view.webview.paint();

    let rect = DeviceIntRect::from_size(DeviceIntSize::new(w as i32, h as i32));

    if let Some(image_buf) = rendering_context.read_to_image(rect) {
        let pixels = image_buf.into_raw();
        view.last_frame = ImageInfo::new(pixels, PixelFormat::Rgba, w, h);
    } else {
        log::warn!(
            "iced_webview: servo read_to_image returned no data ({w}x{h}); keeping previous frame"
        );
    }

    view.needs_render = false;
}

impl Engine for Servo {
    fn handles_urls(&self) -> bool {
        true
    }

    fn update(&mut self) {
        self.instance.spin_event_loop();

        for view in self.views.values_mut() {
            // Drain delegate state
            if let Some(url) = view.delegate_state.url.borrow_mut().take() {
                view.url = url;
            }
            if let Some(title) = view.delegate_state.title.borrow_mut().take() {
                view.title = title;
            }
            {
                let cursor = *view.delegate_state.cursor.borrow();
                view.cursor = cursor_to_interaction(cursor);
            }
            if view.delegate_state.frame_ready.replace(false) {
                view.needs_render = true;
            }
        }
    }

    fn render(&mut self) {
        let rc = Rc::clone(&self.rendering_context);
        let scale = self.scale_factor;
        for view in self.views.values_mut() {
            if view.needs_render {
                capture_frame(view, &rc, scale);
            }
        }
    }

    fn request_render(&mut self, id: ViewId) {
        let rc = Rc::clone(&self.rendering_context);
        let scale = self.scale_factor;
        let Some(view) = self.views.get_mut(id) else {
            return;
        };
        if view.needs_render {
            capture_frame(view, &rc, scale);
        }
    }

    fn new_view(&mut self, size: Size<u32>, content: Option<PageType>) -> ViewId {
        let w = size.width.max(1);
        let h = size.height.max(1);
        let size = Size::new(w, h);

        let delegate_state = Rc::new(DelegateState {
            url: RefCell::new(None),
            title: RefCell::new(None),
            cursor: RefCell::new(Cursor::Default),
            frame_ready: RefCell::new(false),
        });

        let delegate = Rc::new(ViewDelegate {
            state: Rc::clone(&delegate_state),
        });

        let (url_str, initial_url) = match &content {
            Some(PageType::Url(u)) => (u.clone(), Url::parse(u).ok()),
            Some(PageType::Html(html)) => {
                let data_url =
                    format!("data:text/html;charset=utf-8,{}", urlencoding::encode(html));
                (String::new(), Url::parse(&data_url).ok())
            }
            None => (String::new(), None),
        };

        let mut builder = WebViewBuilder::new(
            &self.instance,
            Rc::clone(&self.rendering_context) as Rc<dyn servo::RenderingContext>,
        )
        .delegate(delegate as Rc<dyn WebViewDelegate>);

        if let Some(url) = initial_url {
            builder = builder.url(url);
        }

        let webview = builder.build();
        webview.focus();
        webview.show();
        webview.set_hidpi_scale_factor(
            euclid::Scale::<f32, DeviceIndependentPixel, DevicePixel>::new(self.scale_factor),
        );
        let phys = physical_size(size, self.scale_factor);
        self.rendering_context.resize(phys);
        webview.resize(phys);

        let view = ServoView {
            webview,
            delegate_state,
            url: url_str,
            title: String::new(),
            cursor: Interaction::Idle,
            last_frame: ImageInfo::blank(phys.width, phys.height),
            needs_render: true,
            size,
            last_cursor: DevicePoint::new(phys.width as f32 / 2.0, phys.height as f32 / 2.0),
        };
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
        let phys = physical_size(size, self.scale_factor);
        self.rendering_context.resize(phys);
        for view in self.views.values_mut() {
            view.size = size;
            view.webview.resize(phys);
            view.needs_render = true;
        }
    }

    fn set_scale_factor(&mut self, scale: f32) {
        if (self.scale_factor - scale).abs() < f32::EPSILON {
            return;
        }
        self.scale_factor = scale;
        for view in self.views.values_mut() {
            view.webview.set_hidpi_scale_factor(euclid::Scale::<
                f32,
                DeviceIndependentPixel,
                DevicePixel,
            >::new(scale));
            let phys = physical_size(view.size, scale);
            self.rendering_context.resize(phys);
            view.webview.resize(phys);
            view.needs_render = true;
        }
    }

    fn handle_keyboard_event(&mut self, id: ViewId, event: keyboard::Event) {
        let Some(view) = self.views.get_mut(id) else {
            return;
        };
        if let Some(kb) = iced_keyboard_to_servo(event) {
            view.webview.notify_input_event(InputEvent::Keyboard(kb));
        }
    }

    fn handle_mouse_event(&mut self, id: ViewId, point: Point, event: mouse::Event) {
        let device_point =
            DevicePoint::new(point.x * self.scale_factor, point.y * self.scale_factor);
        let Some(view) = self.views.get_mut(id) else {
            return;
        };
        view.last_cursor = device_point;

        match event {
            mouse::Event::ButtonPressed(button) => {
                if let Some(servo_btn) = iced_button_to_servo(button) {
                    view.webview
                        .notify_input_event(InputEvent::MouseButton(MouseButtonEvent {
                            action: MouseButtonAction::Down,
                            button: servo_btn,
                            point: WebViewPoint::Device(device_point),
                        }));
                }
            }
            mouse::Event::ButtonReleased(button) => {
                if let Some(servo_btn) = iced_button_to_servo(button) {
                    view.webview
                        .notify_input_event(InputEvent::MouseButton(MouseButtonEvent {
                            action: MouseButtonAction::Up,
                            button: servo_btn,
                            point: WebViewPoint::Device(device_point),
                        }));
                }
            }
            mouse::Event::CursorMoved { .. } => {
                view.webview
                    .notify_input_event(InputEvent::MouseMove(MouseMoveEvent {
                        point: WebViewPoint::Device(device_point),
                        is_compatibility_event_for_touch: false,
                    }));
            }
            mouse::Event::WheelScrolled { delta } => {
                let _ = view;
                self.scroll(id, delta);
            }
            _ => {}
        }
    }

    fn scroll(&mut self, id: ViewId, delta: mouse::ScrollDelta) {
        let scale = self.scale_factor;
        let Some(view) = self.views.get_mut(id) else {
            return;
        };
        let (dx, dy, mode) = match delta {
            mouse::ScrollDelta::Lines { x, y } => (x as f64, y as f64, WheelMode::DeltaLine),
            mouse::ScrollDelta::Pixels { x, y } => (
                (x * scale) as f64,
                (y * scale) as f64,
                WheelMode::DeltaPixel,
            ),
        };
        let cursor_point = view.last_cursor;
        view.webview
            .notify_input_event(InputEvent::Wheel(WheelEvent {
                delta: WheelDelta {
                    x: dx,
                    y: dy,
                    z: 0.0,
                    mode,
                },
                point: WebViewPoint::Device(cursor_point),
            }));
    }

    fn goto(&mut self, id: ViewId, page_type: PageType) {
        let Some(view) = self.views.get_mut(id) else {
            return;
        };
        match page_type {
            PageType::Url(url) => {
                if let Ok(parsed) = Url::parse(&url) {
                    view.url = url;
                    view.webview.load(parsed);
                }
            }
            PageType::Html(html) => {
                let data_url = format!(
                    "data:text/html;charset=utf-8,{}",
                    urlencoding::encode(&html)
                );
                if let Ok(parsed) = Url::parse(&data_url) {
                    view.webview.load(parsed);
                }
            }
        }
    }

    fn refresh(&mut self, id: ViewId) {
        if let Some(view) = self.views.get(id) {
            view.webview.reload();
        }
    }

    fn go_forward(&mut self, id: ViewId) {
        if let Some(view) = self.views.get(id) {
            view.webview.go_forward(1);
        }
    }

    fn go_back(&mut self, id: ViewId) {
        if let Some(view) = self.views.get(id) {
            view.webview.go_back(1);
        }
    }

    fn get_url(&self, id: ViewId) -> String {
        let Some(view) = self.views.get(id) else {
            return "about:blank".to_string();
        };
        if let Some(url) = view.webview.url() {
            url.to_string()
        } else if view.url.is_empty() {
            "about:blank".to_string()
        } else {
            view.url.clone()
        }
    }

    fn get_title(&self, id: ViewId) -> String {
        let Some(view) = self.views.get(id) else {
            return String::new();
        };
        view.webview
            .page_title()
            .unwrap_or_else(|| view.title.clone())
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
}

fn iced_button_to_servo(button: mouse::Button) -> Option<ServoMouseButton> {
    match button {
        mouse::Button::Left => Some(ServoMouseButton::Left),
        mouse::Button::Right => Some(ServoMouseButton::Right),
        mouse::Button::Middle => Some(ServoMouseButton::Middle),
        mouse::Button::Back => Some(ServoMouseButton::Back),
        mouse::Button::Forward => Some(ServoMouseButton::Forward),
        mouse::Button::Other(n) => Some(ServoMouseButton::Other(n)),
    }
}

fn iced_keyboard_to_servo(event: keyboard::Event) -> Option<KeyboardEvent> {
    use keyboard_types_servo::{KeyState, Modifiers};

    let (state, key, modifiers) = match event {
        keyboard::Event::KeyPressed {
            key: iced_key,
            modifiers: mods,
            ..
        } => (KeyState::Down, iced_key, mods),
        keyboard::Event::KeyReleased {
            key: iced_key,
            modifiers: mods,
            ..
        } => (KeyState::Up, iced_key, mods),
        _ => return None,
    };

    let kt_key = iced_key_to_keyboard_types(&key)?;

    let mut kt_mods = Modifiers::empty();
    if modifiers.shift() {
        kt_mods |= Modifiers::SHIFT;
    }
    if modifiers.control() {
        kt_mods |= Modifiers::CONTROL;
    }
    if modifiers.alt() {
        kt_mods |= Modifiers::ALT;
    }
    if modifiers.logo() {
        kt_mods |= Modifiers::META;
    }

    let kb_event = keyboard_types_servo::KeyboardEvent {
        state,
        key: kt_key,
        code: keyboard_types_servo::Code::Unidentified,
        location: keyboard_types_servo::Location::Standard,
        modifiers: kt_mods,
        repeat: false,
        is_composing: false,
    };

    Some(KeyboardEvent { event: kb_event })
}

fn iced_key_to_keyboard_types(key: &keyboard::Key) -> Option<keyboard_types_servo::Key> {
    use keyboard::key::Named;
    use keyboard_types_servo::NamedKey;
    match key {
        keyboard::Key::Character(s) => Some(keyboard_types_servo::Key::Character(s.to_string())),
        keyboard::Key::Named(named) => {
            let k = match named {
                Named::Enter => keyboard_types_servo::Key::Named(NamedKey::Enter),
                Named::Tab => keyboard_types_servo::Key::Named(NamedKey::Tab),
                Named::Space => keyboard_types_servo::Key::Character(" ".to_string()),
                Named::Backspace => keyboard_types_servo::Key::Named(NamedKey::Backspace),
                Named::Delete => keyboard_types_servo::Key::Named(NamedKey::Delete),
                Named::Escape => keyboard_types_servo::Key::Named(NamedKey::Escape),
                Named::Insert => keyboard_types_servo::Key::Named(NamedKey::Insert),
                Named::CapsLock => keyboard_types_servo::Key::Named(NamedKey::CapsLock),
                Named::NumLock => keyboard_types_servo::Key::Named(NamedKey::NumLock),
                Named::ScrollLock => keyboard_types_servo::Key::Named(NamedKey::ScrollLock),
                Named::Pause => keyboard_types_servo::Key::Named(NamedKey::Pause),
                Named::PrintScreen => keyboard_types_servo::Key::Named(NamedKey::PrintScreen),
                Named::ContextMenu => keyboard_types_servo::Key::Named(NamedKey::ContextMenu),
                Named::ArrowDown => keyboard_types_servo::Key::Named(NamedKey::ArrowDown),
                Named::ArrowLeft => keyboard_types_servo::Key::Named(NamedKey::ArrowLeft),
                Named::ArrowRight => keyboard_types_servo::Key::Named(NamedKey::ArrowRight),
                Named::ArrowUp => keyboard_types_servo::Key::Named(NamedKey::ArrowUp),
                Named::End => keyboard_types_servo::Key::Named(NamedKey::End),
                Named::Home => keyboard_types_servo::Key::Named(NamedKey::Home),
                Named::PageDown => keyboard_types_servo::Key::Named(NamedKey::PageDown),
                Named::PageUp => keyboard_types_servo::Key::Named(NamedKey::PageUp),
                Named::F1 => keyboard_types_servo::Key::Named(NamedKey::F1),
                Named::F2 => keyboard_types_servo::Key::Named(NamedKey::F2),
                Named::F3 => keyboard_types_servo::Key::Named(NamedKey::F3),
                Named::F4 => keyboard_types_servo::Key::Named(NamedKey::F4),
                Named::F5 => keyboard_types_servo::Key::Named(NamedKey::F5),
                Named::F6 => keyboard_types_servo::Key::Named(NamedKey::F6),
                Named::F7 => keyboard_types_servo::Key::Named(NamedKey::F7),
                Named::F8 => keyboard_types_servo::Key::Named(NamedKey::F8),
                Named::F9 => keyboard_types_servo::Key::Named(NamedKey::F9),
                Named::F10 => keyboard_types_servo::Key::Named(NamedKey::F10),
                Named::F11 => keyboard_types_servo::Key::Named(NamedKey::F11),
                Named::F12 => keyboard_types_servo::Key::Named(NamedKey::F12),
                _ => return None,
            };
            Some(k)
        }
        _ => None,
    }
}
