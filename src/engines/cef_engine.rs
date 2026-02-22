use std::cell::RefCell;
use std::os::raw::c_int;
use std::rc::Rc;

use iced::keyboard;
use iced::mouse::{self, Interaction};
use iced::{Point, Size};
use rand::Rng;

use super::{Engine, PageType, PixelFormat, ViewId};
use crate::ImageInfo;

// Pull in all CEF types, traits, and macros. The wrap_*! macros reference
// ImplClient, WrapClient, Client, etc. by unqualified name, so a glob
// import is the simplest way to satisfy them.
use cef::args::Args;
use cef::*;

/// Shared mutable state populated by CEF handler callbacks and drained
/// each `update()` tick.
struct SharedState {
    frame_buffer: Option<(Vec<u8>, u32, u32)>,
    url: Option<String>,
    title: Option<String>,
    cursor_type: CursorType,
    size: Size<u32>,
    scale_factor: f32,
}

// -- CEF App handler (single-process + disable GPU for OSR) --

wrap_app! {
    struct OsrApp;

    impl App {
        fn on_before_command_line_processing(
            &self,
            _process_type: Option<&CefString>,
            command_line: Option<&mut CommandLine>,
        ) {
            if let Some(cmd) = command_line {
                // Run everything in a single process. CEF's multi-process
                // model requires resources (.pak, icudtl.dat, GL libs) to
                // be discoverable by subprocesses, which is fragile on
                // non-FHS systems (Guix, Nix). Single-process avoids this.
                cmd.append_switch(Some(&CefString::from("single-process")));
                // Disable hardware GPU — OSR delivers pixels via on_paint.
                cmd.append_switch(Some(&CefString::from("disable-gpu")));
                cmd.append_switch(Some(&CefString::from("disable-gpu-compositing")));
            }
        }
    }
}

// -- CEF handler implementations via wrap macros --

wrap_render_handler! {
    struct OsrRenderHandler {
        shared: Rc<RefCell<SharedState>>,
    }

    impl RenderHandler {
        fn view_rect(&self, _browser: Option<&mut Browser>, rect: Option<&mut Rect>) {
            if let Some(rect) = rect {
                let shared = self.shared.borrow();
                rect.x = 0;
                rect.y = 0;
                rect.width = shared.size.width as c_int;
                rect.height = shared.size.height as c_int;
            }
        }

        fn screen_info(
            &self,
            _browser: Option<&mut Browser>,
            screen_info: Option<&mut ScreenInfo>,
        ) -> c_int {
            if let Some(info) = screen_info {
                let shared = self.shared.borrow();
                info.device_scale_factor = shared.scale_factor;
                return 1;
            }
            0
        }

        fn on_paint(
            &self,
            _browser: Option<&mut Browser>,
            _type_: PaintElementType,
            _dirty_rects: Option<&[Rect]>,
            buffer: *const u8,
            width: c_int,
            height: c_int,
        ) {
            let w = width as usize;
            let h = height as usize;
            let len = w * h * 4;
            let pixels = unsafe { std::slice::from_raw_parts(buffer, len) }.to_vec();
            self.shared.borrow_mut().frame_buffer = Some((pixels, width as u32, height as u32));
        }
    }
}

wrap_display_handler! {
    struct OsrDisplayHandler {
        shared: Rc<RefCell<SharedState>>,
    }

    impl DisplayHandler {
        fn on_address_change(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            url: Option<&CefString>,
        ) {
            if let Some(url) = url {
                self.shared.borrow_mut().url = Some(url.to_string());
            }
        }

        fn on_title_change(
            &self,
            _browser: Option<&mut Browser>,
            title: Option<&CefString>,
        ) {
            if let Some(title) = title {
                self.shared.borrow_mut().title = Some(title.to_string());
            }
        }

        fn on_cursor_change(
            &self,
            _browser: Option<&mut Browser>,
            _cursor: std::os::raw::c_ulong,
            type_: CursorType,
            _custom_cursor_info: Option<&CursorInfo>,
        ) -> c_int {
            self.shared.borrow_mut().cursor_type = type_;
            0
        }
    }
}

wrap_life_span_handler! {
    struct OsrLifeSpanHandler {
        shared: Rc<RefCell<SharedState>>,
    }

    impl LifeSpanHandler {
        fn on_after_created(&self, _browser: Option<&mut Browser>) {}
        fn on_before_close(&self, _browser: Option<&mut Browser>) {}
    }
}

wrap_client! {
    struct OsrClient {
        render_handler: RenderHandler,
        display_handler: DisplayHandler,
        life_span_handler: LifeSpanHandler,
    }

    impl Client {
        fn render_handler(&self) -> Option<RenderHandler> {
            Some(self.render_handler.clone())
        }

        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(self.display_handler.clone())
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(self.life_span_handler.clone())
        }
    }
}

struct CefView {
    id: ViewId,
    browser: Browser,
    shared: Rc<RefCell<SharedState>>,
    url: String,
    title: String,
    cursor: Interaction,
    last_frame: ImageInfo,
    needs_render: bool,
    size: Size<u32>,
}

/// Full browser engine backed by [CEF/Chromium](https://github.com/tauri-apps/cef-rs)
/// (HTML5, CSS3, JS).
///
/// CEF handles its own networking, scrolling, and JavaScript execution.
/// Rendering is off-screen (windowless), producing BGRA pixel buffers that
/// are uploaded to a persistent GPU texture via iced's shader widget.
///
/// ## Subprocess requirement
///
/// CEF launches helper sub-processes using the same binary. Call
/// [`cef_subprocess_check`] at the very top of `main()` — if it returns
/// `true`, the process is a CEF subprocess and should exit immediately.
///
/// ```rust,ignore
/// fn main() -> iced::Result {
///     if iced_webview::cef_subprocess_check() {
///         return Ok(());
///     }
///     // ... iced application setup ...
/// }
/// ```
pub struct Cef {
    views: Vec<CefView>,
    scale_factor: f32,
    initialized: bool,
}

impl Default for Cef {
    fn default() -> Self {
        let _ = api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
        let args = Args::new();

        // Use XDG cache dir or /tmp as root_cache_path to avoid the
        // "unintended process singleton behavior" warning.
        let cache_dir = std::env::var("XDG_CACHE_HOME").unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            format!("{home}/.cache")
        });
        let cef_cache = format!("{cache_dir}/iced_webview_cef");
        let _ = std::fs::create_dir_all(&cef_cache);

        // Point CEF to its distribution directory so subprocesses can find
        // libEGL.so, libGLESv2.so, .pak resources, etc. Without this, the
        // GPU process fails to initialize and crashes.
        let cef_dir = cef::sys::get_cef_dir().expect("CEF distribution directory not found");
        let cef_dir_str = cef_dir.to_string_lossy();

        let locales_dir = cef_dir.join("locales");
        let locales_str = locales_dir.to_string_lossy();

        let settings = Settings {
            windowless_rendering_enabled: 1,
            external_message_pump: 1,
            no_sandbox: 1,
            root_cache_path: CefString::from(cef_cache.as_str()),
            framework_dir_path: CefString::from(cef_dir_str.as_ref()),
            resources_dir_path: CefString::from(cef_dir_str.as_ref()),
            locales_dir_path: CefString::from(locales_str.as_ref()),
            ..Default::default()
        };

        let mut app = OsrApp::new();

        let result = initialize(
            Some(args.as_main_args()),
            Some(&settings),
            Some(&mut app),
            std::ptr::null_mut(),
        );

        Self {
            views: Vec::new(),
            scale_factor: 1.0,
            initialized: result == 1,
        }
    }
}

/// Ensure the CEF distribution directory is on `LD_LIBRARY_PATH` so that
/// subprocesses (GPU, renderer, utility) can find `libEGL.so`,
/// `libGLESv2.so`, and other CEF shared libraries at runtime.
fn ensure_cef_lib_path() {
    if let Some(cef_dir) = cef::sys::get_cef_dir() {
        let cef_dir_str = cef_dir.to_string_lossy().to_string();
        let ld_path = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
        if !ld_path.contains(&cef_dir_str) {
            let new_path = if ld_path.is_empty() {
                cef_dir_str
            } else {
                format!("{cef_dir_str}:{ld_path}")
            };
            unsafe { std::env::set_var("LD_LIBRARY_PATH", new_path) };
        }
    }
}

/// Check if the current process is a CEF subprocess.
///
/// Must be called at the very top of `main()`. Returns `true` if this
/// process is a CEF helper (renderer, GPU, utility) — in that case,
/// exit immediately without starting the iced application.
pub fn cef_subprocess_check() -> bool {
    ensure_cef_lib_path();
    let _ = api_hash(cef::sys::CEF_API_VERSION_LAST, 0);
    let args = Args::new();

    let cmd_line = args.as_cmd_line();
    let is_browser = if let Some(cmd) = &cmd_line {
        let switch = CefString::from("type");
        cmd.has_switch(Some(&switch)) != 1
    } else {
        true
    };

    let mut app = OsrApp::new();
    let ret = execute_process(Some(args.as_main_args()), Some(&mut app), std::ptr::null_mut());

    if is_browser {
        false
    } else {
        ret >= 0
    }
}

impl Cef {
    fn find_view(&self, id: ViewId) -> &CefView {
        self.views
            .iter()
            .find(|v| v.id == id)
            .expect("The requested View id was not found")
    }

    fn find_view_mut(&mut self, id: ViewId) -> &mut CefView {
        self.views
            .iter_mut()
            .find(|v| v.id == id)
            .expect("The requested View id was not found")
    }
}

fn cursor_type_to_interaction(cursor: CursorType) -> Interaction {
    match cursor {
        CursorType::POINTER => Interaction::Pointer,
        CursorType::IBEAM => Interaction::Text,
        CursorType::CROSS => Interaction::Crosshair,
        CursorType::HAND => Interaction::Pointer,
        CursorType::GRAB => Interaction::Grab,
        CursorType::GRABBING => Interaction::Grabbing,
        CursorType::NOTALLOWED => Interaction::NotAllowed,
        CursorType::EASTWESTRESIZE
        | CursorType::EASTRESIZE
        | CursorType::WESTRESIZE
        | CursorType::COLUMNRESIZE => Interaction::ResizingHorizontally,
        CursorType::NORTHSOUTHRESIZE
        | CursorType::NORTHRESIZE
        | CursorType::SOUTHRESIZE
        | CursorType::ROWRESIZE => Interaction::ResizingVertically,
        CursorType::ZOOMIN => Interaction::ZoomIn,
        CursorType::ZOOMOUT => Interaction::ZoomOut,
        _ => Interaction::Idle,
    }
}

impl Engine for Cef {
    fn handles_urls(&self) -> bool {
        true
    }

    fn update(&mut self) {
        if !self.initialized {
            return;
        }

        do_message_loop_work();

        for view in &mut self.views {
            let mut shared = view.shared.borrow_mut();

            if let Some((pixels, w, h)) = shared.frame_buffer.take() {
                view.last_frame = ImageInfo::new(pixels, PixelFormat::Bgra, w, h);
                view.needs_render = false;
            }
            if let Some(url) = shared.url.take() {
                view.url = url;
            }
            if let Some(title) = shared.title.take() {
                view.title = title;
            }
            view.cursor = cursor_type_to_interaction(shared.cursor_type);
        }
    }

    fn render(&mut self, _size: Size<u32>) {
        // CEF renders asynchronously via on_paint — nothing to do here.
    }

    fn request_render(&mut self, _id: ViewId, _size: Size<u32>) {
        // CEF renders asynchronously via on_paint — nothing to do here.
    }

    fn new_view(&mut self, size: Size<u32>, content: Option<PageType>) -> ViewId {
        let id = rand::thread_rng().gen();
        let w = size.width.max(1);
        let h = size.height.max(1);
        let size = Size::new(w, h);

        let shared = Rc::new(RefCell::new(SharedState {
            frame_buffer: None,
            url: None,
            title: None,
            cursor_type: CursorType::POINTER,
            size,
            scale_factor: self.scale_factor,
        }));

        let render_handler = OsrRenderHandler::new(Rc::clone(&shared));
        let display_handler = OsrDisplayHandler::new(Rc::clone(&shared));
        let life_span_handler = OsrLifeSpanHandler::new(Rc::clone(&shared));
        let mut client = OsrClient::new(render_handler, display_handler, life_span_handler);

        let window_info = WindowInfo::default().set_as_windowless(0);

        let browser_settings = BrowserSettings {
            windowless_frame_rate: 60,
            ..Default::default()
        };

        let url_str = match &content {
            Some(PageType::Url(u)) => u.clone(),
            _ => String::new(),
        };

        let initial_url: CefString = match &content {
            Some(PageType::Url(u)) => CefString::from(u.as_str()),
            Some(PageType::Html(html)) => {
                let data_url = format!(
                    "data:text/html;charset=utf-8,{}",
                    urlencoding::encode(html)
                );
                CefString::from(data_url.as_str())
            }
            None => CefString::from("about:blank"),
        };

        let browser = browser_host_create_browser_sync(
            Some(&window_info),
            Some(&mut client),
            Some(&initial_url),
            Some(&browser_settings),
            None,
            None,
        )
        .expect("CEF browser creation failed");

        let view = CefView {
            id,
            browser,
            shared,
            url: url_str,
            title: String::new(),
            cursor: Interaction::Idle,
            last_frame: ImageInfo::blank(w, h),
            needs_render: true,
            size,
        };
        self.views.push(view);
        id
    }

    fn remove_view(&mut self, id: ViewId) {
        if let Some(pos) = self.views.iter().position(|v| v.id == id) {
            let view = &self.views[pos];
            if let Some(host) = view.browser.host() {
                host.close_browser(1);
            }
            self.views.remove(pos);
        }
    }

    fn has_view(&self, id: ViewId) -> bool {
        self.views.iter().any(|v| v.id == id)
    }

    fn view_ids(&self) -> Vec<ViewId> {
        self.views.iter().map(|v| v.id).collect()
    }

    fn focus(&mut self) {
        if let Some(view) = self.views.last() {
            if let Some(host) = view.browser.host() {
                host.set_focus(1);
            }
        }
    }

    fn unfocus(&self) {
        if let Some(view) = self.views.last() {
            if let Some(host) = view.browser.host() {
                host.set_focus(0);
            }
        }
    }

    fn resize(&mut self, size: Size<u32>) {
        let w = size.width.max(1);
        let h = size.height.max(1);
        let new_size = Size::new(w, h);
        for view in &mut self.views {
            view.size = new_size;
            view.shared.borrow_mut().size = new_size;
            if let Some(host) = view.browser.host() {
                host.was_resized();
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
            view.shared.borrow_mut().scale_factor = scale;
            if let Some(host) = view.browser.host() {
                host.notify_screen_info_changed();
                host.was_resized();
            }
            view.needs_render = true;
        }
    }

    fn handle_keyboard_event(&mut self, id: ViewId, event: keyboard::Event) {
        let view = self.find_view_mut(id);
        if let Some(host) = view.browser.host() {
            if let Some(ke) = iced_keyboard_to_cef(event) {
                host.send_key_event(Some(&ke));
            }
        }
    }

    fn handle_mouse_event(&mut self, id: ViewId, point: Point, event: mouse::Event) {
        let view = self.find_view_mut(id);
        let Some(host) = view.browser.host() else {
            return;
        };

        let me = MouseEvent {
            x: point.x as c_int,
            y: point.y as c_int,
            modifiers: 0,
        };

        match event {
            mouse::Event::ButtonPressed(button) => {
                if let Some(cef_btn) = iced_button_to_cef(button) {
                    host.send_mouse_click_event(Some(&me), cef_btn, 0, 1);
                }
            }
            mouse::Event::ButtonReleased(button) => {
                if let Some(cef_btn) = iced_button_to_cef(button) {
                    host.send_mouse_click_event(Some(&me), cef_btn, 1, 1);
                }
            }
            mouse::Event::CursorMoved { .. } => {
                host.send_mouse_move_event(Some(&me), 0);
            }
            mouse::Event::WheelScrolled { delta } => {
                drop(host);
                self.scroll(id, delta);
            }
            mouse::Event::CursorLeft => {
                host.send_mouse_move_event(Some(&me), 1);
            }
            _ => {}
        }
    }

    fn scroll(&mut self, id: ViewId, delta: mouse::ScrollDelta) {
        let view = self.find_view_mut(id);
        let Some(host) = view.browser.host() else {
            return;
        };

        let me = MouseEvent {
            x: 0,
            y: 0,
            modifiers: 0,
        };

        let (dx, dy) = match delta {
            mouse::ScrollDelta::Lines { x, y } => {
                ((x * 40.0) as c_int, (y * 40.0) as c_int)
            }
            mouse::ScrollDelta::Pixels { x, y } => (x as c_int, y as c_int),
        };

        host.send_mouse_wheel_event(Some(&me), dx, dy);
    }

    fn goto(&mut self, id: ViewId, page_type: PageType) {
        let view = self.find_view_mut(id);
        let Some(frame) = view.browser.main_frame() else {
            return;
        };

        match page_type {
            PageType::Url(url) => {
                view.url = url.clone();
                let cef_url = CefString::from(url.as_str());
                frame.load_url(Some(&cef_url));
            }
            PageType::Html(html) => {
                let data_url = format!(
                    "data:text/html;charset=utf-8,{}",
                    urlencoding::encode(&html)
                );
                let cef_url = CefString::from(data_url.as_str());
                frame.load_url(Some(&cef_url));
            }
        }
    }

    fn refresh(&mut self, id: ViewId) {
        self.find_view(id).browser.reload();
    }

    fn go_forward(&mut self, id: ViewId) {
        self.find_view(id).browser.go_forward();
    }

    fn go_back(&mut self, id: ViewId) {
        self.find_view(id).browser.go_back();
    }

    fn get_url(&self, id: ViewId) -> String {
        let view = self.find_view(id);
        if let Some(frame) = view.browser.main_frame() {
            let url_userfree = frame.url();
            let url_cef: CefString = CefString::from(&url_userfree);
            let s = url_cef.to_string();
            if !s.is_empty() {
                return s;
            }
        }
        if view.url.is_empty() {
            "about:blank".to_string()
        } else {
            view.url.clone()
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
}

fn iced_button_to_cef(button: mouse::Button) -> Option<MouseButtonType> {
    match button {
        mouse::Button::Left => Some(MouseButtonType::LEFT),
        mouse::Button::Middle => Some(MouseButtonType::MIDDLE),
        mouse::Button::Right => Some(MouseButtonType::RIGHT),
        _ => None,
    }
}

fn iced_keyboard_to_cef(event: keyboard::Event) -> Option<KeyEvent> {
    let (key_type, key, modifiers) = match event {
        keyboard::Event::KeyPressed {
            key: iced_key,
            modifiers: mods,
            ..
        } => (KeyEventType::RAWKEYDOWN, iced_key, mods),
        keyboard::Event::KeyReleased {
            key: iced_key,
            modifiers: mods,
            ..
        } => (KeyEventType::KEYUP, iced_key, mods),
        _ => return None,
    };

    let (windows_key_code, character) = iced_key_to_cef(&key)?;

    // Event flag constants: SHIFT=2, CONTROL=4, ALT=8
    let mut cef_modifiers: u32 = 0;
    if modifiers.shift() {
        cef_modifiers |= 2;
    }
    if modifiers.control() {
        cef_modifiers |= 4;
    }
    if modifiers.alt() {
        cef_modifiers |= 8;
    }

    Some(KeyEvent {
        size: std::mem::size_of::<KeyEvent>(),
        type_: key_type,
        modifiers: cef_modifiers,
        windows_key_code: windows_key_code as c_int,
        native_key_code: 0,
        is_system_key: 0,
        character,
        unmodified_character: character,
        focus_on_editable_field: 0,
    })
}

fn iced_key_to_cef(key: &keyboard::Key) -> Option<(i32, u16)> {
    use keyboard::key::Named;

    match key {
        keyboard::Key::Character(s) => {
            let ch = s.chars().next()?;
            let vk = if ch.is_ascii_alphabetic() {
                ch.to_ascii_uppercase() as i32
            } else {
                ch as i32
            };
            Some((vk, ch as u16))
        }
        keyboard::Key::Named(named) => {
            let (vk, ch) = match named {
                Named::Enter => (0x0D, 0x0D),
                Named::Tab => (0x09, 0x09),
                Named::Space => (0x20, 0x20),
                Named::Backspace => (0x08, 0x08),
                Named::Delete => (0x2E, 0),
                Named::Escape => (0x1B, 0x1B),
                Named::Insert => (0x2D, 0),
                Named::Home => (0x24, 0),
                Named::End => (0x23, 0),
                Named::PageUp => (0x21, 0),
                Named::PageDown => (0x22, 0),
                Named::ArrowUp => (0x26, 0),
                Named::ArrowDown => (0x28, 0),
                Named::ArrowLeft => (0x25, 0),
                Named::ArrowRight => (0x27, 0),
                Named::F1 => (0x70, 0),
                Named::F2 => (0x71, 0),
                Named::F3 => (0x72, 0),
                Named::F4 => (0x73, 0),
                Named::F5 => (0x74, 0),
                Named::F6 => (0x75, 0),
                Named::F7 => (0x76, 0),
                Named::F8 => (0x77, 0),
                Named::F9 => (0x78, 0),
                Named::F10 => (0x79, 0),
                Named::F11 => (0x7A, 0),
                Named::F12 => (0x7B, 0),
                _ => return None,
            };
            Some((vk, ch))
        }
        _ => None,
    }
}
