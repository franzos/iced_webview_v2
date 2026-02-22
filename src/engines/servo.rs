use std::cell::RefCell;
use std::rc::Rc;

use iced::keyboard;
use iced::mouse::{self, Interaction};
use iced::{Point, Size};
use rand::Rng;

use super::{Engine, PageType, PixelFormat, ViewId};
use crate::ImageInfo;

use servo::{
    Cursor, InputEvent, KeyboardEvent, MouseButton as ServoMouseButton, MouseButtonAction,
    MouseButtonEvent, MouseMoveEvent, RenderingContext, Servo as ServoInstance, ServoBuilder,
    SoftwareRenderingContext, WebView, WebViewBuilder, WebViewDelegate, WheelDelta, WheelEvent,
    WheelMode,
};
use servo::{DeviceIndependentPixel, DeviceIntRect, DeviceIntSize, DevicePixel, DevicePoint, WebViewPoint};
use dpi::PhysicalSize;
use url::Url;

/// No-op waker — the iced subscription tick already drives `spin_event_loop`.
struct NoOpWaker;

impl servo::EventLoopWaker for NoOpWaker {
    fn clone_box(&self) -> Box<dyn servo::EventLoopWaker> {
        Box::new(NoOpWaker)
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
    id: ViewId,
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
    views: Vec<ServoView>,
    scale_factor: f32,
}

impl Default for Servo {
    fn default() -> Self {
        let size = PhysicalSize::new(ImageInfo::WIDTH, ImageInfo::HEIGHT);
        let rendering_context = SoftwareRenderingContext::new(size)
            .expect("failed to create SoftwareRenderingContext");
        let rendering_context = Rc::new(rendering_context);

        let instance = ServoBuilder::default()
            .event_loop_waker(Box::new(NoOpWaker))
            .build();

        Self {
            instance,
            rendering_context,
            views: Vec::new(),
            scale_factor: 1.0,
        }
    }
}

impl Servo {
    fn find_view(&self, id: ViewId) -> &ServoView {
        self.views
            .iter()
            .find(|v| v.id == id)
            .expect("The requested View id was not found")
    }

    fn find_view_mut(&mut self, id: ViewId) -> &mut ServoView {
        self.views
            .iter_mut()
            .find(|v| v.id == id)
            .expect("The requested View id was not found")
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

/// Paint a webview and capture the pixel buffer into `ImageInfo`.
fn capture_frame(view: &mut ServoView, rendering_context: &SoftwareRenderingContext) {
    let w = view.size.width;
    let h = view.size.height;
    if w == 0 || h == 0 {
        return;
    }

    view.webview.paint();

    let rect = DeviceIntRect::from_size(DeviceIntSize::new(w as i32, h as i32));

    if let Some(image_buf) = rendering_context.read_to_image(rect) {
        let pixels = image_buf.into_raw();
        view.last_frame = ImageInfo::new(pixels, PixelFormat::Rgba, w, h);
    }

    view.needs_render = false;
}

impl Engine for Servo {
    fn handles_urls(&self) -> bool {
        true
    }

    fn update(&mut self) {
        self.instance.spin_event_loop();

        for view in &mut self.views {
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

    fn render(&mut self, _size: Size<u32>) {
        for i in 0..self.views.len() {
            if self.views[i].needs_render {
                let rc = Rc::clone(&self.rendering_context);
                capture_frame(&mut self.views[i], &rc);
            }
        }
    }

    fn request_render(&mut self, id: ViewId, _size: Size<u32>) {
        let rc = Rc::clone(&self.rendering_context);
        let view = self.find_view_mut(id);
        if view.needs_render {
            capture_frame(view, &rc);
        }
    }

    fn new_view(&mut self, size: Size<u32>, content: Option<PageType>) -> ViewId {
        let id = rand::thread_rng().gen();
        let w = size.width.max(1);
        let h = size.height.max(1);
        let size = Size::new(w, h);

        self.rendering_context
            .resize(PhysicalSize::new(w, h));

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
                let data_url = format!(
                    "data:text/html;charset=utf-8,{}",
                    urlencoding::encode(html)
                );
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

        let view = ServoView {
            id,
            webview,
            delegate_state,
            url: url_str,
            title: String::new(),
            cursor: Interaction::Idle,
            last_frame: ImageInfo::blank(w, h),
            needs_render: true,
            size,
            last_cursor: DevicePoint::new(w as f32 / 2.0, h as f32 / 2.0),
        };
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

    fn focus(&mut self) {
        if let Some(view) = self.views.last() {
            view.webview.focus();
        }
    }

    fn unfocus(&self) {
        if let Some(view) = self.views.last() {
            view.webview.blur();
        }
    }

    fn resize(&mut self, size: Size<u32>) {
        let phys = PhysicalSize::new(size.width.max(1), size.height.max(1));
        self.rendering_context.resize(phys);
        for view in &mut self.views {
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
        for view in &mut self.views {
            view.webview.set_hidpi_scale_factor(
                euclid::Scale::<f32, DeviceIndependentPixel, DevicePixel>::new(scale),
            );
            view.needs_render = true;
        }
    }

    fn handle_keyboard_event(&mut self, id: ViewId, event: keyboard::Event) {
        let view = self.find_view_mut(id);
        if let Some(kb) = iced_keyboard_to_servo(event) {
            view.webview
                .notify_input_event(InputEvent::Keyboard(kb));
        }
    }

    fn handle_mouse_event(&mut self, id: ViewId, point: Point, event: mouse::Event) {
        let device_point = DevicePoint::new(point.x, point.y);
        self.find_view_mut(id).last_cursor = device_point;

        match event {
            mouse::Event::ButtonPressed(button) => {
                if let Some(servo_btn) = iced_button_to_servo(button) {
                    self.find_view_mut(id)
                        .webview
                        .notify_input_event(InputEvent::MouseButton(MouseButtonEvent {
                            action: MouseButtonAction::Down,
                            button: servo_btn,
                            point: WebViewPoint::Device(device_point),
                        }));
                }
            }
            mouse::Event::ButtonReleased(button) => {
                if let Some(servo_btn) = iced_button_to_servo(button) {
                    self.find_view_mut(id)
                        .webview
                        .notify_input_event(InputEvent::MouseButton(MouseButtonEvent {
                            action: MouseButtonAction::Up,
                            button: servo_btn,
                            point: WebViewPoint::Device(device_point),
                        }));
                }
            }
            mouse::Event::CursorMoved { .. } => {
                self.find_view_mut(id)
                    .webview
                    .notify_input_event(InputEvent::MouseMove(MouseMoveEvent {
                        point: WebViewPoint::Device(device_point),
                        is_compatibility_event_for_touch: false,
                    }));
            }
            mouse::Event::WheelScrolled { delta } => {
                self.scroll(id, delta);
            }
            _ => {}
        }
    }

    fn scroll(&mut self, id: ViewId, delta: mouse::ScrollDelta) {
        let view = self.find_view_mut(id);
        let (dx, dy, mode) = match delta {
            mouse::ScrollDelta::Lines { x, y } => {
                (x as f64, y as f64, WheelMode::DeltaLine)
            }
            mouse::ScrollDelta::Pixels { x, y } => {
                (x as f64, y as f64, WheelMode::DeltaPixel)
            }
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
        let view = self.find_view_mut(id);
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
        self.find_view(id).webview.reload();
    }

    fn go_forward(&mut self, id: ViewId) {
        self.find_view(id).webview.go_forward(1);
    }

    fn go_back(&mut self, id: ViewId) {
        self.find_view(id).webview.go_back(1);
    }

    fn get_url(&self, id: ViewId) -> String {
        let view = self.find_view(id);
        if let Some(url) = view.webview.url() {
            url.to_string()
        } else if view.url.is_empty() {
            "about:blank".to_string()
        } else {
            view.url.clone()
        }
    }

    fn get_title(&self, id: ViewId) -> String {
        let view = self.find_view(id);
        view.webview
            .page_title()
            .unwrap_or_else(|| view.title.clone())
    }

    fn get_cursor(&self, id: ViewId) -> Interaction {
        self.find_view(id).cursor
    }

    fn get_view(&self, id: ViewId) -> &ImageInfo {
        &self.find_view(id).last_frame
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
