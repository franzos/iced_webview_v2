use iced::{Element, Subscription, Task};
use iced_webview::{Action, PageType, WebView};

#[cfg(not(feature = "servo"))]
use iced::time;
#[cfg(not(feature = "servo"))]
use std::time::Duration;

#[cfg(feature = "cef")]
type Engine = iced_webview::Cef;
#[cfg(all(feature = "servo", not(feature = "cef")))]
type Engine = iced_webview::Servo;
#[cfg(all(feature = "blitz", not(feature = "servo"), not(feature = "cef")))]
type Engine = iced_webview::Blitz;
#[cfg(all(
    feature = "litehtml",
    not(feature = "blitz"),
    not(feature = "servo"),
    not(feature = "cef")
))]
type Engine = iced_webview::Litehtml;

static URL: &str = "https://docs.rs/iced/latest/iced/index.html";

fn main() -> iced::Result {
    #[cfg(feature = "cef")]
    if iced_webview::cef_subprocess_check() {
        return Ok(());
    }

    iced::application(App::new, App::update, App::view)
        .title("Web view")
        .subscription(App::subscription)
        .run()
}

#[derive(Debug, Clone)]
enum Message {
    WebView(Action),
    ViewCreated,
    WindowChanged(iced::window::Id),
    ScaleFactor(f32),
}

struct App {
    webview: WebView<Engine, Message>,
    ready: bool,
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let webview = WebView::new()
            .on_create_view(Message::ViewCreated)
            .on_action(Message::WebView);
        (
            Self {
                webview,
                ready: false,
            },
            Task::done(Message::WebView(Action::CreateView(PageType::Url(
                URL.to_string(),
            )))),
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WebView(msg) => self.webview.update(msg),
            Message::ViewCreated => {
                self.ready = true;
                self.webview.update(Action::ChangeView(0))
            }
            Message::WindowChanged(id) => iced::window::scale_factor(id).map(Message::ScaleFactor),
            Message::ScaleFactor(f) => {
                self.webview.set_scale_factor(f);
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        if self.ready {
            self.webview.view().map(Message::WebView)
        } else {
            iced::widget::text("Loading...").into()
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        let window = Subscription::batch([
            iced::window::open_events().map(Message::WindowChanged),
            iced::window::resize_events().map(|(id, _)| Message::WindowChanged(id)),
        ]);

        #[cfg(feature = "servo")]
        let engine = self.webview.subscription().map(Message::WebView);
        #[cfg(not(feature = "servo"))]
        let engine = time::every(Duration::from_millis(10))
            .map(|_| Action::Update)
            .map(Message::WebView);

        Subscription::batch([window, engine])
    }
}
