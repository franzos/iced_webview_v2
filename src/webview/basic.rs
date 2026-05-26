use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use iced::advanced::image as core_image;
use iced::advanced::{
    self, layout,
    renderer::{self},
    widget::Tree,
    Clipboard, Layout, Shell, Widget,
};
use iced::keyboard;
use iced::mouse::{self, Interaction};
use iced::{Element, Point, Size, Task};
use iced::{Event, Length, Rectangle};
use url::Url;

use crate::{engines, ImageInfo, PageType, ViewId};

#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq)]
/// Handles Actions for Basic webview
pub enum Action {
    /// Changes view to the desired view index
    ChangeView(u32),
    /// Closes current window & makes last used view the current one
    CloseCurrentView,
    /// Closes specific view index
    CloseView(u32),
    /// Creates a new view and makes its index view + 1
    CreateView(PageType),
    GoBackward,
    GoForward,
    GoToUrl(Url),
    Refresh,
    SendKeyboardEvent(keyboard::Event),
    SendMouseEvent(mouse::Event, Point),
    /// Allows users to control when the browser engine proccesses interactions in subscriptions
    Update,
    Resize(Size<u32>),
    /// Copy the current text selection to clipboard
    CopySelection,
    /// Internal: carries the result of a URL fetch for engines without native URL support.
    /// On success returns `(html, css_cache)`.
    FetchComplete(
        ViewId,
        String,
        Result<(String, HashMap<String, String>), String>,
    ),
    /// Internal: carries the result of an image fetch.
    /// The bool is `redraw_on_ready` — when true, the image doesn't affect
    /// layout so `doc.render()` can be skipped (redraw only).
    /// The u64 is the navigation epoch — stale results are discarded.
    ImageFetchComplete(ViewId, String, Result<Vec<u8>, String>, bool, u64),
    /// Internal: carries the window scale factor queried from iced.
    SetScaleFactor(f32),
}

/// The Basic WebView widget that creates and shows webview(s).
///
/// **Important:** You must drive the webview with a periodic [`Action::Update`]
/// subscription (e.g. via `iced::time::every`). Without it the webview will
/// never render and the screen stays blank.
///
/// ```rust,ignore
/// fn subscription(&self) -> iced::Subscription<Message> {
///     iced::time::every(std::time::Duration::from_millis(16))
///         .map(|_| Message::WebView(Action::Update))
/// }
/// ```
pub struct WebView<Engine, Message>
where
    Engine: engines::Engine,
{
    engine: Engine,
    view_size: Size<u32>,
    scale_factor: f32,
    current_view_index: Option<usize>, // the index corresponding to the view_ids list of ViewIds
    view_ids: Vec<ViewId>, // allow users to index by simple id like 0 or 1 instead of a true id
    on_close_view: Option<Message>,
    on_create_view: Option<Message>,
    on_url_change: Option<Box<dyn Fn(String) -> Message>>,
    url: String,
    on_title_change: Option<Box<dyn Fn(String) -> Message>>,
    title: String,
    on_copy: Option<Box<dyn Fn(String) -> Message>>,
    action_mapper: Option<Arc<dyn Fn(Action) -> Message + Send + Sync>>,
    /// Number of image fetches currently in flight. Staged images are only
    /// flushed (triggering an expensive redraw) once this reaches zero, so
    /// a burst of images causes only one redraw instead of one per image.
    inflight_images: usize,
    /// Per-view navigation epoch. Incremented on `GoToUrl` so that image
    /// fetches spawned for a previous page are discarded when they complete.
    nav_epochs: HashMap<ViewId, u64>,
    /// Window scale factor observed by the shader path (f32 bits; `0` = unset).
    /// Read back in the `Update` handler to auto-correct HiDPI rendering.
    scale_observer: Arc<AtomicU32>,
}

impl<Engine: engines::Engine + Default, Message: Send + Clone + 'static> WebView<Engine, Message> {
    fn get_current_view_id(&self) -> Option<ViewId> {
        self.current_view_index
            .and_then(|idx| self.view_ids.get(idx))
            .copied()
    }

    fn index_as_view_id(&self, index: u32) -> Option<usize> {
        self.view_ids.get(index as usize).copied()
    }
}

impl<Engine: engines::Engine + Default, Message: Send + Clone + 'static> Default
    for WebView<Engine, Message>
{
    fn default() -> Self {
        WebView {
            engine: Engine::default(),
            view_size: Size {
                width: 1920,
                height: 1080,
            },
            scale_factor: 1.0,
            current_view_index: None,
            view_ids: Vec::new(),
            on_close_view: None,
            on_create_view: None,
            on_url_change: None,
            url: String::new(),
            on_title_change: None,
            title: String::new(),
            on_copy: None,
            action_mapper: None,
            inflight_images: 0,
            nav_epochs: HashMap::new(),
            scale_observer: Arc::new(AtomicU32::new(0)),
        }
    }
}

impl<Engine: engines::Engine + Default, Message: Send + Clone + 'static> WebView<Engine, Message> {
    /// Create new basic WebView widget
    pub fn new() -> Self {
        Self::default()
    }

    /// Override the display scale factor for HiDPI rendering.
    /// The engine renders at `logical_size * scale_factor` pixels. The library
    /// auto-detects the window scale factor, so calling this is only needed to
    /// force a specific value.
    pub fn set_scale_factor(&mut self, scale: f32) {
        if (self.scale_factor - scale).abs() <= f32::EPSILON {
            return;
        }
        self.scale_factor = scale;
        self.engine.set_scale_factor(scale);
    }

    fn query_scale_factor(&self) -> Task<Message> {
        if let Some(mapper) = &self.action_mapper {
            let mapper = mapper.clone();
            iced::window::latest()
                .and_then(iced::window::scale_factor)
                .map(move |f| mapper(Action::SetScaleFactor(f)))
        } else {
            Task::none()
        }
    }

    /// subscribe to create view events
    pub fn on_create_view(mut self, on_create_view: Message) -> Self {
        self.on_create_view = Some(on_create_view);
        self
    }

    /// subscribe to close view events
    pub fn on_close_view(mut self, on_close_view: Message) -> Self {
        self.on_close_view = Some(on_close_view);
        self
    }

    /// subscribe to url change events
    pub fn on_url_change(mut self, on_url_change: impl Fn(String) -> Message + 'static) -> Self {
        self.on_url_change = Some(Box::new(on_url_change));
        self
    }

    /// subscribe to title change events
    pub fn on_title_change(
        mut self,
        on_title_change: impl Fn(String) -> Message + 'static,
    ) -> Self {
        self.on_title_change = Some(Box::new(on_title_change));
        self
    }

    /// Subscribe to copy events (text selection copied via Ctrl+C / Cmd+C)
    pub fn on_copy(mut self, on_copy: impl Fn(String) -> Message + 'static) -> Self {
        self.on_copy = Some(Box::new(on_copy));
        self
    }

    /// Provide a mapper from [`Action`] to `Message` so the webview can spawn
    /// async tasks that route back through the iced update loop. **Required**
    /// for litehtml and blitz engines — without it, URL navigation and image
    /// loading will not work.
    pub fn on_action(mut self, mapper: impl Fn(Action) -> Message + Send + Sync + 'static) -> Self {
        self.action_mapper = Some(Arc::new(mapper));
        self
    }

    /// Set the initial viewport size used before the first resize event.
    /// Defaults to 1920x1080.
    pub fn with_initial_size(mut self, size: Size<u32>) -> Self {
        self.view_size = size;
        self
    }

    /// Passes update to webview
    pub fn update(&mut self, action: Action) -> Task<Message> {
        let mut tasks = Vec::new();

        if let Some(view_id) = self.get_current_view_id() {
            if let Some(on_url_change) = &self.on_url_change {
                let url = self.engine.get_url(view_id);
                if self.url != url {
                    tasks.push(Task::done(on_url_change(url.clone())));
                    self.url = url;
                }
            }
            if let Some(on_title_change) = &self.on_title_change {
                let title = self.engine.get_title(view_id);
                if self.title != title {
                    tasks.push(Task::done(on_title_change(title.clone())));
                    self.title = title;
                }
            }
        }

        match action {
            Action::ChangeView(index) => {
                if let Some(view_id) = self.index_as_view_id(index) {
                    self.current_view_index = Some(index as usize);
                    self.engine.request_render(view_id, self.view_size);
                    tasks.push(self.query_scale_factor());
                } else {
                    eprintln!(
                        "iced_webview: ChangeView index {} is invalid or already closed",
                        index
                    );
                }
            }
            Action::CloseCurrentView => {
                if let Some(idx) = self.current_view_index {
                    if let Some(view_id) = self.get_current_view_id() {
                        self.engine.remove_view(view_id);
                        self.view_ids.remove(idx);
                        self.current_view_index = None;
                        if let Some(on_view_close) = &self.on_close_view {
                            tasks.push(Task::done(on_view_close.clone()));
                        }
                    } else {
                        eprintln!(
                            "iced_webview: CloseCurrentView failed — view index {} is stale",
                            idx
                        );
                        self.current_view_index = None;
                    }
                }
            }
            Action::CloseView(index) => {
                if let Some(view_id) = self.index_as_view_id(index) {
                    self.engine.remove_view(view_id);
                    self.view_ids.remove(index as usize);

                    // Adjust current_view_index after removal
                    if let Some(current) = self.current_view_index {
                        if current == index as usize {
                            self.current_view_index = None;
                        } else if current > index as usize {
                            self.current_view_index = Some(current - 1);
                        }
                    }

                    if let Some(on_view_close) = &self.on_close_view {
                        tasks.push(Task::done(on_view_close.clone()))
                    }
                } else {
                    eprintln!(
                        "iced_webview: CloseView index {} is invalid or already closed",
                        index
                    );
                }
            }
            Action::CreateView(page_type) => {
                if let PageType::Url(url) = page_type {
                    if !self.engine.handles_urls() {
                        let id = self.engine.new_view(self.view_size, None);
                        self.view_ids.push(id);
                        self.engine.goto(id, PageType::Url(url.clone()));

                        #[cfg(any(feature = "litehtml", feature = "blitz"))]
                        if let Some(mapper) = &self.action_mapper {
                            let mapper = mapper.clone();
                            let url_clone = url.clone();
                            tasks.push(Task::perform(
                                crate::fetch::fetch_html(url),
                                move |result| mapper(Action::FetchComplete(id, url_clone, result)),
                            ));
                        } else {
                            eprintln!("iced_webview: .on_action() is required for URL navigation and image loading when the engine does not handle URLs natively. Call .on_action(Message::YourVariant) on your WebView builder.");
                        }

                        #[cfg(not(any(feature = "litehtml", feature = "blitz")))]
                        eprintln!("iced_webview: .on_action() is required for URL navigation and image loading when the engine does not handle URLs natively. Call .on_action(Message::YourVariant) on your WebView builder.");
                    } else {
                        let id = self
                            .engine
                            .new_view(self.view_size, Some(PageType::Url(url)));
                        self.view_ids.push(id);
                    }
                } else {
                    let id = self.engine.new_view(self.view_size, Some(page_type));
                    self.view_ids.push(id);
                }

                if let Some(on_view_create) = &self.on_create_view {
                    tasks.push(Task::done(on_view_create.clone()))
                }
                tasks.push(self.query_scale_factor());
            }
            Action::GoBackward => {
                if let Some(view_id) = self.get_current_view_id() {
                    self.engine.go_back(view_id);
                }
            }
            Action::GoForward => {
                if let Some(view_id) = self.get_current_view_id() {
                    self.engine.go_forward(view_id);
                }
            }
            Action::GoToUrl(url) => {
                if let Some(view_id) = self.get_current_view_id() {
                    self.inflight_images = 0;
                    let epoch = self.nav_epochs.entry(view_id).or_insert(0);
                    *epoch = epoch.wrapping_add(1);
                    let url_str = url.to_string();
                    self.engine.goto(view_id, PageType::Url(url_str.clone()));

                    #[cfg(any(feature = "litehtml", feature = "blitz"))]
                    if !self.engine.handles_urls() {
                        if let Some(mapper) = &self.action_mapper {
                            let mapper = mapper.clone();
                            let fetch_url = url_str.clone();
                            tasks.push(Task::perform(
                                crate::fetch::fetch_html(fetch_url),
                                move |result| {
                                    mapper(Action::FetchComplete(view_id, url_str, result))
                                },
                            ));
                        } else {
                            eprintln!("iced_webview: .on_action() is required for URL navigation and image loading when the engine does not handle URLs natively. Call .on_action(Message::YourVariant) on your WebView builder.");
                        }
                    }

                    #[cfg(not(any(feature = "litehtml", feature = "blitz")))]
                    if !self.engine.handles_urls() {
                        eprintln!("iced_webview: .on_action() is required for URL navigation and image loading when the engine does not handle URLs natively. Call .on_action(Message::YourVariant) on your WebView builder.");
                    }
                }
            }
            Action::Refresh => {
                if let Some(view_id) = self.get_current_view_id() {
                    self.engine.refresh(view_id);
                }
            }
            Action::SendKeyboardEvent(event) => {
                if let Some(view_id) = self.get_current_view_id() {
                    self.engine.handle_keyboard_event(view_id, event);
                }
            }
            Action::SendMouseEvent(event, point) => {
                if let Some(view_id) = self.get_current_view_id() {
                    self.engine.handle_mouse_event(view_id, point, event);

                    // Check if the click triggered an anchor navigation
                    if let Some(href) = self.engine.take_anchor_click(view_id) {
                        let current = self.engine.get_url(view_id);
                        let base = Url::parse(&current).ok();
                        match Url::parse(&href).or_else(|_| {
                            base.as_ref()
                                .ok_or(url::ParseError::RelativeUrlWithoutBase)
                                .and_then(|b| b.join(&href))
                        }) {
                            Ok(resolved) => {
                                let scheme = resolved.scheme();
                                if scheme == "http" || scheme == "https" {
                                    let is_same_page = base.as_ref().is_some_and(|cur| {
                                        crate::util::is_same_page(&resolved, cur)
                                    });
                                    if is_same_page {
                                        if let Some(fragment) = resolved.fragment() {
                                            self.engine.scroll_to_fragment(view_id, fragment);
                                        }
                                    } else {
                                        tasks.push(self.update(Action::GoToUrl(resolved)));
                                    }
                                }
                            }
                            Err(e) => {
                                eprintln!(
                                    "iced_webview: failed to resolve anchor URL '{href}': {e}"
                                );
                            }
                        }
                    }
                }

                // Don't request_render here — the periodic Update tick handles
                // it. Re-rendering inline on every mouse event (especially
                // scroll) creates a new image Handle each time, causing GPU
                // texture churn and visible gray flashes.
                return Task::batch(tasks);
            }
            Action::Update => {
                self.engine.update();

                let observed = self.scale_observer.load(Ordering::Relaxed);
                if observed != 0 {
                    self.set_scale_factor(f32::from_bits(observed));
                }

                if let Some(view_id) = self.get_current_view_id() {
                    self.engine.request_render(view_id, self.view_size);

                    // Flush staged images only when all fetches are done,
                    // so the entire batch is drawn in one pass.
                    if self.inflight_images == 0 {
                        self.engine.flush_staged_images(view_id, self.view_size);
                    }
                }

                // Discover images that need fetching after layout
                #[cfg(any(feature = "litehtml", feature = "blitz"))]
                if let Some(mapper) = &self.action_mapper {
                    let pending = self.engine.take_pending_images();
                    for (view_id, src, baseurl, redraw_on_ready) in pending {
                        let page_url = self.engine.get_url(view_id);
                        // Resolve against the baseurl context (e.g. stylesheet URL),
                        // falling back to the page URL.
                        let resolved = crate::util::resolve_url(&src, &baseurl, &page_url);
                        let resolved = match resolved {
                            Ok(u) => u,
                            Err(_) => continue,
                        };
                        let scheme = resolved.scheme();
                        if scheme != "http" && scheme != "https" {
                            continue;
                        }
                        self.inflight_images += 1;
                        let mapper = mapper.clone();
                        let raw_src = src.clone();
                        let epoch = *self.nav_epochs.get(&view_id).unwrap_or(&0);
                        tasks.push(Task::perform(
                            crate::fetch::fetch_image(resolved.to_string()),
                            move |result| {
                                mapper(Action::ImageFetchComplete(
                                    view_id,
                                    raw_src,
                                    result,
                                    redraw_on_ready,
                                    epoch,
                                ))
                            },
                        ));
                    }
                }

                return Task::batch(tasks);
            }
            Action::Resize(size) => {
                if self.view_size != size {
                    self.view_size = size;
                    self.engine.resize(size);
                    tasks.push(self.query_scale_factor());
                } else {
                    // No-op resize (published every frame because the widget
                    // is recreated with bounds 0,0). Skip request_render to
                    // avoid texture churn during scrolling.
                    return Task::batch(tasks);
                }
            }
            Action::CopySelection => {
                if let Some(view_id) = self.get_current_view_id() {
                    if let Some(text) = self.engine.get_selected_text(view_id) {
                        if let Some(on_copy) = &self.on_copy {
                            tasks.push(Task::done((on_copy)(text)));
                        }
                    }
                }
                return Task::batch(tasks);
            }
            Action::FetchComplete(view_id, url, result) => {
                if !self.engine.has_view(view_id) {
                    return Task::batch(tasks);
                }
                match result {
                    Ok((html, css_cache)) => {
                        self.engine.set_css_cache(view_id, css_cache);
                        self.engine.goto(view_id, PageType::Html(html));
                    }
                    Err(e) => {
                        let error_html = format!(
                            "<html><body><h1>Failed to load</h1><p>{}</p><p>{}</p></body></html>",
                            crate::util::html_escape(&url),
                            crate::util::html_escape(&e),
                        );
                        self.engine.goto(view_id, PageType::Html(error_html));
                    }
                }
            }
            Action::ImageFetchComplete(view_id, src, result, redraw_on_ready, epoch) => {
                self.inflight_images = self.inflight_images.saturating_sub(1);
                let current_epoch = *self.nav_epochs.get(&view_id).unwrap_or(&0);
                if epoch != current_epoch {
                    // Stale fetch from a previous navigation — discard.
                    return Task::batch(tasks);
                }
                if self.engine.has_view(view_id) {
                    match &result {
                        Ok(bytes) => {
                            self.engine.load_image_from_bytes(
                                view_id,
                                &src,
                                bytes,
                                redraw_on_ready,
                            );
                        }
                        Err(e) => {
                            eprintln!("iced_webview: failed to fetch image '{}': {}", src, e);
                        }
                    }
                }
                // Don't call request_render here — the periodic Update tick
                // picks up staged images via request_render's staged check.
                return Task::batch(tasks);
            }
            Action::SetScaleFactor(f) => {
                self.set_scale_factor(f);
            }
        };

        if let Some(view_id) = self.get_current_view_id() {
            self.engine.request_render(view_id, self.view_size);
        }

        Task::batch(tasks)
    }

    /// Returns webview widget for the current view
    pub fn view<'a, T: 'a>(&'a self) -> Element<'a, Action, T> {
        let id = match self.get_current_view_id() {
            Some(id) => id,
            None => return iced::widget::Column::new().into(),
        };
        let content_height = self.engine.get_content_height(id);

        if content_height > 0.0 {
            // litehtml renders a full-document buffer: draw it with the image
            // Handle widget and scroll by y-offset. (blitz/servo report height 0
            // and take the shader path below.)
            WebViewWidget::new(
                self.engine.get_view(id),
                self.engine.get_cursor(id),
                self.engine.get_selection_rects(id),
                self.engine.get_scroll_y(id),
                content_height,
            )
            .into()
        } else {
            // Engines that manage their own scrolling and produce a viewport-
            // sized frame each tick (servo, blitz, cef): use the shader widget
            // for direct GPU texture updates, avoiding Handle cache churn.
            #[cfg(any(feature = "servo", feature = "cef", feature = "blitz"))]
            {
                use crate::webview::shader_widget::WebViewShaderProgram;
                iced::widget::Shader::new(WebViewShaderProgram::new(
                    self.engine.get_view(id),
                    self.engine.get_cursor(id),
                    self.scale_observer.clone(),
                ))
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
            }
            #[cfg(not(any(feature = "servo", feature = "cef", feature = "blitz")))]
            {
                WebViewWidget::new(
                    self.engine.get_view(id),
                    self.engine.get_cursor(id),
                    self.engine.get_selection_rects(id),
                    0.0,
                    0.0,
                )
                .into()
            }
        }
    }

    /// Get the current view's image info for direct rendering
    pub fn current_image(&self) -> Option<&crate::ImageInfo> {
        self.get_current_view_id()
            .map(|id| self.engine.get_view(id))
    }

    /// Get the current view's URL
    pub fn current_url(&self) -> &str {
        &self.url
    }

    /// Get the current view's title
    pub fn current_title(&self) -> &str {
        &self.title
    }
}

#[cfg(feature = "servo")]
impl<Message: Send + Clone + 'static> WebView<crate::engines::servo::Servo, Message> {
    /// Event-driven subscription for the Servo engine — yields
    /// [`Action::Update`] whenever Servo wakes the embedder, with a 500ms
    /// fallback tick. Use this in place of a hardcoded `time::every(...)`
    /// timer when running with the `servo` feature.
    pub fn subscription(&self) -> iced::Subscription<Action> {
        self.engine.subscription()
    }
}

struct WebViewWidget<'a> {
    handle: core_image::Handle,
    cursor: Interaction,
    bounds: Size<u32>,
    selection_rects: &'a [[f32; 4]],
    scroll_y: f32,
    content_height: f32,
}

impl<'a> WebViewWidget<'a> {
    fn new(
        image_info: &ImageInfo,
        cursor: Interaction,
        selection_rects: &'a [[f32; 4]],
        scroll_y: f32,
        content_height: f32,
    ) -> Self {
        Self {
            handle: image_info.as_handle(),
            cursor,
            bounds: Size::new(0, 0),
            selection_rects,
            scroll_y,
            content_height,
        }
    }
}

impl<'a, Renderer, Theme> Widget<Action, Theme, Renderer> for WebViewWidget<'a>
where
    Renderer: iced::advanced::Renderer
        + iced::advanced::image::Renderer<Handle = iced::advanced::image::Handle>,
{
    fn size(&self) -> Size<Length> {
        Size {
            width: Length::Fill,
            height: Length::Fill,
        }
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::Node::new(limits.max())
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();

        if self.content_height > 0.0 {
            // Draw rect is in logical coords; iced scales it to physical by the
            // window scale factor, matching the physically-sized pixel buffer.
            // content_height and scroll_y are logical — no scale applied here.
            renderer.with_layer(bounds, |renderer| {
                let image_bounds = Rectangle {
                    x: bounds.x,
                    y: bounds.y - self.scroll_y,
                    width: bounds.width,
                    height: self.content_height,
                };
                renderer.draw_image(
                    core_image::Image::new(self.handle.clone())
                        .snap(true)
                        .filter_method(core_image::FilterMethod::Nearest),
                    image_bounds,
                    *viewport,
                );
            });
        } else {
            renderer.draw_image(
                core_image::Image::new(self.handle.clone())
                    .snap(true)
                    .filter_method(core_image::FilterMethod::Nearest),
                bounds,
                *viewport,
            );
        }

        // Selection highlights — stored in document coordinates,
        // offset by scroll_y to match the scrolled content image.
        if !self.selection_rects.is_empty() {
            let rects = self.selection_rects;
            let scroll_y = self.scroll_y;
            renderer.with_layer(bounds, |renderer| {
                let highlight = iced::Color::from_rgba(0.26, 0.52, 0.96, 0.3);
                for rect in rects {
                    let quad_bounds = Rectangle {
                        x: bounds.x + rect[0],
                        y: bounds.y + rect[1] - scroll_y,
                        width: rect[2],
                        height: rect[3],
                    };
                    renderer.fill_quad(
                        renderer::Quad {
                            bounds: quad_bounds,
                            ..renderer::Quad::default()
                        },
                        highlight,
                    );
                }
            });
        }
    }

    fn update(
        &mut self,
        _state: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Action>,
        _viewport: &Rectangle,
    ) {
        let size = Size::new(
            layout.bounds().width.round() as u32,
            layout.bounds().height.round() as u32,
        );
        if self.bounds != size {
            self.bounds = size;
            shell.publish(Action::Resize(size));
        }

        match event {
            Event::Keyboard(event) => {
                if let keyboard::Event::KeyPressed {
                    key: keyboard::Key::Character(c),
                    modifiers,
                    ..
                } = event
                {
                    if modifiers.command() && c.as_str() == "c" {
                        shell.publish(Action::CopySelection);
                    }
                }
                shell.publish(Action::SendKeyboardEvent(event.clone()));
            }
            Event::Mouse(event) => {
                if let Some(point) = cursor.position_in(layout.bounds()) {
                    shell.publish(Action::SendMouseEvent(*event, point));
                } else if matches!(event, mouse::Event::CursorLeft) {
                    shell.publish(Action::SendMouseEvent(*event, Point::ORIGIN));
                }
            }
            _ => (),
        }
    }

    fn mouse_interaction(
        &self,
        _state: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &Renderer,
    ) -> mouse::Interaction {
        if cursor.is_over(layout.bounds()) {
            self.cursor
        } else {
            mouse::Interaction::Idle
        }
    }
}

impl<'a, Message: 'a, Renderer, Theme> From<WebViewWidget<'a>>
    for Element<'a, Message, Theme, Renderer>
where
    Renderer: advanced::Renderer + advanced::image::Renderer<Handle = advanced::image::Handle>,
    WebViewWidget<'a>: Widget<Message, Theme, Renderer>,
{
    fn from(widget: WebViewWidget<'a>) -> Self {
        Self::new(widget)
    }
}
