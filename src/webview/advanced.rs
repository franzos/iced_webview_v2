use std::collections::HashMap;
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
pub enum Action {
    CloseView(ViewId),
    CreateView(PageType),
    GoBackward(ViewId),
    GoForward(ViewId),
    GoToUrl(ViewId, Url),
    Refresh(ViewId),
    SendKeyboardEvent(ViewId, keyboard::Event),
    SendMouseEvent(ViewId, mouse::Event, Point),
    /// Call this periodically to update a view
    Update(ViewId),
    /// Call this periodically to update a view(s)
    UpdateAll,
    Resize(Size<u32>),
    /// Copy the current text selection to clipboard
    CopySelection(ViewId),
    /// Internal: carries the result of a URL fetch for engines without native URL support.
    /// On success returns `(html, css_cache)`.
    FetchComplete(
        ViewId,
        String,
        Result<(String, HashMap<String, String>), String>,
    ),
    /// Internal: carries the result of an image fetch.
    /// The bool is `redraw_on_ready`, the u64 is the navigation epoch.
    ImageFetchComplete(ViewId, String, Result<Vec<u8>, String>, bool, u64),
}

/// The Advanced WebView widget that creates and shows webview(s)
pub struct WebView<Engine, Message>
where
    Engine: engines::Engine,
{
    engine: Engine,
    view_size: Size<u32>,
    scale_factor: f32,
    on_close_view: Option<Box<dyn Fn(ViewId) -> Message>>,
    on_create_view: Option<Box<dyn Fn(ViewId) -> Message>>,
    on_url_change: Option<Box<dyn Fn(ViewId, String) -> Message>>,
    urls: Vec<(ViewId, String)>,
    on_title_change: Option<Box<dyn Fn(ViewId, String) -> Message>>,
    titles: Vec<(ViewId, String)>,
    on_copy: Option<Box<dyn Fn(String) -> Message>>,
    action_mapper: Option<Arc<dyn Fn(Action) -> Message + Send + Sync>>,
    inflight_images: usize,
    nav_epochs: HashMap<ViewId, u64>,
}

impl<Engine: engines::Engine + Default, Message: Send + Clone + 'static> Default
    for WebView<Engine, Message>
{
    fn default() -> Self {
        WebView {
            engine: Engine::default(),
            view_size: Size::new(1920, 1080),
            scale_factor: 1.0,
            on_close_view: None,
            on_create_view: None,
            on_url_change: None,
            urls: Vec::new(),
            on_title_change: None,
            titles: Vec::new(),
            on_copy: None,
            action_mapper: None,
            inflight_images: 0,
            nav_epochs: HashMap::new(),
        }
    }
}

impl<Engine: engines::Engine + Default, Message: Send + Clone + 'static> WebView<Engine, Message> {
    /// Create new Advanced Webview widget
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the display scale factor for HiDPI rendering.
    pub fn set_scale_factor(&mut self, scale: f32) {
        self.scale_factor = scale;
        self.engine.set_scale_factor(scale);
    }

    /// Subscribe to create view events
    pub fn on_create_view(mut self, on_create_view: impl Fn(usize) -> Message + 'static) -> Self {
        self.on_create_view = Some(Box::new(on_create_view));
        self
    }

    /// Subscribe to close view events
    pub fn on_close_view(mut self, on_close_view: impl Fn(usize) -> Message + 'static) -> Self {
        self.on_close_view = Some(Box::new(on_close_view));
        self
    }

    /// Subscribe to url change events
    pub fn on_url_change(
        mut self,
        on_url_change: impl Fn(ViewId, String) -> Message + 'static,
    ) -> Self {
        self.on_url_change = Some(Box::new(on_url_change));
        self
    }

    /// Subscribe to title change events
    pub fn on_title_change(
        mut self,
        on_title_change: impl Fn(ViewId, String) -> Message + 'static,
    ) -> Self {
        self.on_title_change = Some(Box::new(on_title_change));
        self
    }

    /// Subscribe to copy events (text selection copied via Ctrl+C / Cmd+C)
    pub fn on_copy(mut self, on_copy: impl Fn(String) -> Message + 'static) -> Self {
        self.on_copy = Some(Box::new(on_copy));
        self
    }

    /// Provide a mapper from Action to Message so the webview can spawn async
    /// tasks (e.g. URL fetches) that route back through the update loop.
    /// Required for URL navigation on engines that don't handle URLs natively.
    pub fn on_action(mut self, mapper: impl Fn(Action) -> Message + Send + Sync + 'static) -> Self {
        self.action_mapper = Some(Arc::new(mapper));
        self
    }

    /// Passes update to webview
    pub fn update(&mut self, action: Action) -> Task<Message> {
        let mut tasks = Vec::new();

        // Check url & title for changes and callback if so
        for (id, url) in self.urls.iter_mut() {
            if let Some(on_url_change) = &self.on_url_change {
                let engine_url = self.engine.get_url(*id);
                if *url != engine_url {
                    *url = engine_url.clone();
                    tasks.push(Task::done(on_url_change(*id, engine_url)));
                }
            }
        }
        for (id, title) in self.titles.iter_mut() {
            if let Some(on_title_change) = &self.on_title_change {
                let engine_title = self.engine.get_title(*id);
                if *title != engine_title {
                    *title = engine_title.clone();
                    tasks.push(Task::done(on_title_change(*id, engine_title)));
                }
            }
        }

        match action {
            Action::CloseView(id) => {
                self.engine.remove_view(id);
                self.urls.retain(|url| url.0 != id);
                self.titles.retain(|title| title.0 != id);

                if let Some(on_view_close) = &self.on_close_view {
                    tasks.push(Task::done((on_view_close)(id)))
                }
            }
            Action::CreateView(page_type) => {
                let id = if let PageType::Url(url) = page_type {
                    if !self.engine.handles_urls() {
                        let id = self.engine.new_view(self.view_size, None);
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
                            eprintln!("iced_webview: on_action() mapper required for URL navigation with this engine");
                        }

                        #[cfg(not(any(feature = "litehtml", feature = "blitz")))]
                        eprintln!("iced_webview: on_action() mapper required for URL navigation with this engine");

                        id
                    } else {
                        self.engine
                            .new_view(self.view_size, Some(PageType::Url(url)))
                    }
                } else {
                    self.engine.new_view(self.view_size, Some(page_type))
                };

                self.urls.push((id, String::new()));
                self.titles.push((id, String::new()));

                if let Some(on_view_create) = &self.on_create_view {
                    tasks.push(Task::done((on_view_create)(id)))
                }
            }
            Action::GoBackward(id) => {
                self.engine.go_back(id);
                self.engine.request_render(id, self.view_size);
            }
            Action::GoForward(id) => {
                self.engine.go_forward(id);
                self.engine.request_render(id, self.view_size);
            }
            Action::GoToUrl(id, url) => {
                self.inflight_images = 0;
                let epoch = self.nav_epochs.entry(id).or_insert(0);
                *epoch = epoch.wrapping_add(1);
                let url_str = url.to_string();
                self.engine.goto(id, PageType::Url(url_str.clone()));

                #[cfg(any(feature = "litehtml", feature = "blitz"))]
                if !self.engine.handles_urls() {
                    if let Some(mapper) = &self.action_mapper {
                        let mapper = mapper.clone();
                        let fetch_url = url_str.clone();
                        tasks.push(Task::perform(
                            crate::fetch::fetch_html(fetch_url),
                            move |result| mapper(Action::FetchComplete(id, url_str, result)),
                        ));
                    } else {
                        eprintln!("iced_webview: on_action() mapper required for URL navigation with this engine");
                    }
                }

                #[cfg(not(any(feature = "litehtml", feature = "blitz")))]
                if !self.engine.handles_urls() {
                    eprintln!("iced_webview: on_action() mapper required for URL navigation with this engine");
                }

                self.engine.request_render(id, self.view_size);
            }
            Action::Refresh(id) => {
                self.engine.refresh(id);
                self.engine.request_render(id, self.view_size);
            }
            Action::SendKeyboardEvent(id, event) => {
                self.engine.handle_keyboard_event(id, event);
                self.engine.request_render(id, self.view_size);
            }
            Action::SendMouseEvent(id, event, point) => {
                self.engine.handle_mouse_event(id, point, event);

                if let Some(href) = self.engine.take_anchor_click(id) {
                    let current = self.engine.get_url(id);
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
                                    resolved.scheme() == cur.scheme()
                                        && resolved.host() == cur.host()
                                        && resolved.port() == cur.port()
                                        && resolved.path() == cur.path()
                                        && resolved.query() == cur.query()
                                });
                                if is_same_page {
                                    if let Some(fragment) = resolved.fragment() {
                                        self.engine.scroll_to_fragment(id, fragment);
                                    }
                                } else {
                                    tasks.push(self.update(Action::GoToUrl(id, resolved)));
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("iced_webview: failed to resolve anchor URL '{href}': {e}");
                        }
                    }
                }

                return Task::batch(tasks);
            }
            Action::Update(id) => {
                self.engine.update();
                self.engine.request_render(id, self.view_size);

                if self.inflight_images == 0 {
                    self.engine.flush_staged_images(id, self.view_size);
                }

                #[cfg(any(feature = "litehtml", feature = "blitz"))]
                if let Some(mapper) = &self.action_mapper {
                    let pending = self.engine.take_pending_images();
                    for (view_id, src, baseurl, redraw_on_ready) in pending {
                        let page_url = self.engine.get_url(view_id);
                        let resolved = Url::parse(&src)
                            .or_else(|_| {
                                if !baseurl.is_empty() {
                                    Url::parse(&baseurl).and_then(|b| b.join(&src))
                                } else {
                                    Err(url::ParseError::RelativeUrlWithoutBase)
                                }
                            })
                            .or_else(|_| Url::parse(&page_url).and_then(|base| base.join(&src)));
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
            Action::UpdateAll => {
                self.engine.update();

                if self.inflight_images == 0 {
                    for id in self.engine.view_ids() {
                        self.engine.flush_staged_images(id, self.view_size);
                    }
                }

                self.engine.render(self.view_size);

                #[cfg(any(feature = "litehtml", feature = "blitz"))]
                if let Some(mapper) = &self.action_mapper {
                    let pending = self.engine.take_pending_images();
                    for (view_id, src, baseurl, redraw_on_ready) in pending {
                        let page_url = self.engine.get_url(view_id);
                        let resolved = Url::parse(&src)
                            .or_else(|_| {
                                if !baseurl.is_empty() {
                                    Url::parse(&baseurl).and_then(|b| b.join(&src))
                                } else {
                                    Err(url::ParseError::RelativeUrlWithoutBase)
                                }
                            })
                            .or_else(|_| Url::parse(&page_url).and_then(|base| base.join(&src)));
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
                }
                // Always skip the per-action render below; the Update/UpdateAll
                // tick handles it. For no-op resizes (most frames) this avoids
                // texture churn; for real resizes the next tick picks it up.
                return Task::batch(tasks);
            }
            Action::CopySelection(id) => {
                if let Some(text) = self.engine.get_selected_text(id) {
                    if let Some(on_copy) = &self.on_copy {
                        tasks.push(Task::done((on_copy)(text)));
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
                self.engine.request_render(view_id, self.view_size);
            }
            Action::ImageFetchComplete(view_id, src, result, redraw_on_ready, epoch) => {
                self.inflight_images = self.inflight_images.saturating_sub(1);
                let current_epoch = *self.nav_epochs.get(&view_id).unwrap_or(&0);
                if epoch != current_epoch {
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
                return Task::batch(tasks);
            }
        };

        Task::batch(tasks)
    }

    /// Like a normal `view()` method in iced, but takes an id of the desired view
    pub fn view<'a, T: 'a>(&'a self, id: usize) -> Element<'a, Action, T> {
        WebViewWidget::new(
            id,
            self.view_size,
            self.engine.get_view(id),
            self.engine.get_cursor(id),
            self.engine.get_selection_rects(id),
            self.engine.get_scroll_y(id),
            self.engine.get_content_height(id),
        )
        .into()
    }
}

struct WebViewWidget<'a> {
    id: ViewId,
    bounds: Size<u32>,
    handle: core_image::Handle,
    cursor: Interaction,
    selection_rects: &'a [[f32; 4]],
    scroll_y: f32,
    content_height: f32,
}

impl<'a> WebViewWidget<'a> {
    fn new(
        id: ViewId,
        bounds: Size<u32>,
        image: &ImageInfo,
        cursor: Interaction,
        selection_rects: &'a [[f32; 4]],
        scroll_y: f32,
        content_height: f32,
    ) -> Self {
        Self {
            id,
            bounds,
            handle: image.as_handle(),
            cursor,
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
            renderer.with_layer(bounds, |renderer| {
                let image_bounds = Rectangle {
                    x: bounds.x,
                    y: bounds.y - self.scroll_y,
                    width: bounds.width,
                    height: self.content_height,
                };
                renderer.draw_image(
                    core_image::Image::new(self.handle.clone()).snap(true),
                    image_bounds,
                    *viewport,
                );
            });
        } else {
            renderer.draw_image(
                core_image::Image::new(self.handle.clone()).snap(true),
                bounds,
                *viewport,
            );
        }

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
        let size = Size::new(layout.bounds().width as u32, layout.bounds().height as u32);
        if self.bounds != size {
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
                        shell.publish(Action::CopySelection(self.id));
                    }
                }
                shell.publish(Action::SendKeyboardEvent(self.id, event.clone()));
            }
            Event::Mouse(event) => {
                if let Some(point) = cursor.position_in(layout.bounds()) {
                    shell.publish(Action::SendMouseEvent(self.id, *event, point));
                } else if matches!(event, mouse::Event::CursorLeft) {
                    shell.publish(Action::SendMouseEvent(self.id, *event, Point::ORIGIN));
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
