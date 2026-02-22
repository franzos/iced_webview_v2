use iced::{time, Element, Subscription, Task};
use iced_webview::{Action, PageType, WebView};
use std::time::Duration;

#[cfg(feature = "servo")]
type Engine = iced_webview::Servo;
#[cfg(all(feature = "blitz", not(feature = "servo")))]
type Engine = iced_webview::Blitz;
#[cfg(all(feature = "litehtml", not(feature = "blitz"), not(feature = "servo")))]
type Engine = iced_webview::Litehtml;

static URL: &str = "https://docs.rs/iced/latest/iced/index.html";

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("Web view")
        .subscription(App::subscription)
        .run()
}

#[derive(Debug, Clone)]
enum Message {
    WebView(Action),
    ViewCreated,
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
        time::every(Duration::from_millis(10))
            .map(|_| Action::Update)
            .map(Message::WebView)
    }
}
