use iced::{
    time,
    widget::{column, text},
    Element, Subscription, Task,
};
use iced_webview::{Action, PageType, WebView};
use std::time::Duration;

#[cfg(feature = "servo")]
type Engine = iced_webview::Servo;

#[cfg(all(feature = "blitz", not(feature = "servo")))]
type Engine = iced_webview::Blitz;

#[cfg(all(feature = "litehtml", not(feature = "blitz"), not(feature = "servo")))]
type Engine = iced_webview::Litehtml;

/// Sample email HTML -- table-based layout typical of marketing emails.
static EMAIL_HTML: &str = r##"
<html>
<head>
<style>
    body { margin: 0; padding: 0; background-color: #f4f4f4; font-family: Arial, Helvetica, sans-serif; }
    .wrapper { width: 100%; background-color: #f4f4f4; padding: 20px 0; }
    .container { max-width: 600px; margin: 0 auto; background-color: #ffffff; border-radius: 4px; overflow: hidden; }
    .header { background-color: #2b6cb0; color: #ffffff; padding: 24px; text-align: center; }
    .header h1 { margin: 0; font-size: 22px; }
    .content { padding: 24px; color: #333333; line-height: 1.6; }
    .content h2 { color: #2b6cb0; margin-top: 0; }
    .footer { background-color: #edf2f7; padding: 16px 24px; text-align: center; font-size: 12px; color: #718096; }
    .btn { display: inline-block; background-color: #2b6cb0; color: #ffffff; padding: 10px 24px; text-decoration: none; border-radius: 4px; }
</style>
</head>
<body>
<div class="wrapper">
<table width="100%" cellpadding="0" cellspacing="0" role="presentation">
<tr><td align="center">
    <table class="container" width="600" cellpadding="0" cellspacing="0" role="presentation">
        <tr><td class="header">
            <h1>Monthly Project Update</h1>
        </td></tr>
        <tr><td class="content">
            <h2>Hello there,</h2>
            <p>
                Here's a quick recap of what happened this month. The team shipped
                three new features, squashed a handful of bugs, and improved overall
                rendering performance by roughly 18%.
            </p>
            <p>
                We also started integrating <strong>litehtml</strong> as a lightweight
                alternative for email rendering. It turns out the combination of
                tiny-skia rasterization and a purpose-built email CSS stylesheet
                works pretty well for the kind of table-heavy layouts emails use.
            </p>
            <table width="100%" cellpadding="8" cellspacing="0" style="border-collapse: collapse; margin: 16px 0;">
                <tr style="background-color: #edf2f7;">
                    <td style="border: 1px solid #e2e8f0;"><strong>Metric</strong></td>
                    <td style="border: 1px solid #e2e8f0;"><strong>Before</strong></td>
                    <td style="border: 1px solid #e2e8f0;"><strong>After</strong></td>
                </tr>
                <tr>
                    <td style="border: 1px solid #e2e8f0;">Render time</td>
                    <td style="border: 1px solid #e2e8f0;">42 ms</td>
                    <td style="border: 1px solid #e2e8f0;">34 ms</td>
                </tr>
                <tr>
                    <td style="border: 1px solid #e2e8f0;">Memory usage</td>
                    <td style="border: 1px solid #e2e8f0;">12 MB</td>
                    <td style="border: 1px solid #e2e8f0;">9 MB</td>
                </tr>
            </table>
            <p>
                If you want to take a closer look, the source is on the repo.
                Feel free to open an issue if anything looks off.
            </p>
            <p style="text-align: center; margin: 24px 0;">
                <a class="btn" href="#">View full report</a>
            </p>
            <p>Cheers,<br>The Dev Team</p>
        </td></tr>
        <tr><td class="footer">
            You received this because you're subscribed to project updates.<br>
            <a href="#" style="color: #4a5568;">Unsubscribe</a>
        </td></tr>
    </table>
</td></tr>
</table>
</div>
</body>
</html>
"##;

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("Email Renderer")
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
        let mut webview = WebView::new().on_create_view(Message::ViewCreated);
        webview.set_scale_factor(2.0);
        (
            Self {
                webview,
                ready: false,
            },
            Task::done(Message::WebView(Action::CreateView(PageType::Html(
                EMAIL_HTML.to_string(),
            )))),
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WebView(action) => self.webview.update(action),
            Message::ViewCreated => {
                if !self.ready {
                    self.ready = true;
                    return self.webview.update(Action::ChangeView(0));
                }
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<Message> {
        if !self.ready {
            return column![text("Loading email...")].into();
        }

        column![self.webview.view().map(Message::WebView)].into()
    }

    fn subscription(&self) -> Subscription<Message> {
        time::every(Duration::from_millis(16))
            .map(|_| Action::Update)
            .map(Message::WebView)
    }
}
