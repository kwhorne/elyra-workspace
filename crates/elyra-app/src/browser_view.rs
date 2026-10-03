//! The Browser tab (⇧⌘B): a web page per thread, shown with WebKit, for the
//! app the agent is building. Development servers running in the thread's
//! folder are offered as shortcuts, and agents can look at local pages
//! through the gateway (see `gateway`).
//!
//! The page is a native view over GPUI (see `webview`). It is attached only
//! while the tab is on screen and nothing GPUI draws would sit over it:
//! dialogs, sheets, the command palette or About.

use crate::context_view::{Server, local_servers};
use crate::webview::{self, NativeHost, PageState, WebEvent, WebView};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Sizable as _, Theme, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

/// How often to check whether a dialog or sheet now covers the page.
const COVER_CHECK: Duration = Duration::from_millis(150);

pub struct BrowserView {
    host: Option<Rc<NativeHost>>,
    page: Option<Rc<WebView>>,
    events: async_channel::Sender<WebEvent>,
    address: Entity<InputState>,
    state: PageState,
    cwd: PathBuf,
    servers: Vec<Server>,
    /// The tab is on screen (set by the workspace on every render).
    shown: bool,
    /// The workspace draws something over the page (palette, About).
    covered: bool,
    _tasks: Vec<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

/// Turn what was typed into an address: local servers get `http://`,
/// anything with a dot or a port `https://`, the rest a web search.
pub fn normalize_address(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.contains("://") || text.starts_with("about:") {
        return Some(text.to_string());
    }
    let host = text.split(['/', '?', '#']).next().unwrap_or("");
    let bare = host.split(':').next().unwrap_or("");
    let local = bare == "localhost"
        || bare == "127.0.0.1"
        || bare.ends_with(".localhost")
        || bare.ends_with(".test")
        || bare.ends_with(".local");
    if local {
        return Some(format!("http://{text}"));
    }
    if !text.contains(char::is_whitespace) && (bare.contains('.') || host.contains(':')) {
        return Some(format!("https://{text}"));
    }
    let query: String = text
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            b' ' => "+".into(),
            other => format!("%{other:02X}"),
        })
        .collect();
    Some(format!("https://duckduckgo.com/?q={query}"))
}

impl BrowserView {
    pub fn new(cwd: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let address = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Address or local server, e.g. localhost:5173")
        });
        let (events, rx) = async_channel::unbounded::<WebEvent>();
        let host = NativeHost::new(window).map(Rc::new);
        let subscriptions = vec![cx.subscribe_in(
            &address,
            window,
            |this, _, event: &InputEvent, window, cx| {
                match event {
                    InputEvent::PressEnter { .. } => {
                        let text = this.address.read(cx).value().to_string();
                        if let Some(url) = normalize_address(&text) {
                            this.open(&url, window, cx);
                        }
                    }
                    // Typing goes to GPUI, not the page.
                    InputEvent::Focus => {
                        if let Some(host) = &this.host {
                            host.focus_gpui();
                        }
                    }
                    _ => {}
                }
            },
        )];
        let drain = cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = rx.recv().await {
                let alive = this
                    .update_in(cx, |this, window, cx| this.handle_event(event, window, cx))
                    .is_ok();
                if !alive {
                    break;
                }
            }
        });
        // Detach the page while a dialog or sheet is open over it.
        let cover = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(COVER_CHECK).await;
                let alive = this
                    .update_in(cx, |this, window, cx| this.sync_attachment(window, cx))
                    .is_ok();
                if !alive {
                    break;
                }
            }
        });
        let mut this = Self {
            host,
            page: None,
            events,
            address,
            state: PageState::default(),
            cwd,
            servers: Vec::new(),
            shown: false,
            covered: false,
            _tasks: vec![drain, cover],
            _subscriptions: subscriptions,
        };
        this.scan_servers(cx);
        this
    }

    pub fn set_cwd(&mut self, cwd: PathBuf, cx: &mut Context<Self>) {
        if cwd != self.cwd {
            self.cwd = cwd;
            self.scan_servers(cx);
        }
    }

    fn scan_servers(&mut self, cx: &mut Context<Self>) {
        let root = self.cwd.clone();
        let job = cx
            .background_executor()
            .spawn(async move { local_servers(&root) });
        cx.spawn(async move |this, cx| {
            let servers = job.await;
            let _ = this.update(cx, |this, cx| {
                this.servers = servers;
                cx.notify();
            });
        })
        .detach();
    }

    /// Called by the workspace: whether the Browser tab is on screen, and
    /// whether the workspace draws something over it.
    pub fn set_shown(
        &mut self,
        shown: bool,
        covered: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shown != shown || self.covered != covered {
            self.shown = shown;
            self.covered = covered;
            self.sync_attachment(window, cx);
        }
    }

    fn should_attach(&self, window: &mut Window, cx: &mut App) -> bool {
        self.shown && !self.covered && !window.has_active_dialog(cx) && !window.has_active_sheet(cx)
    }

    fn sync_attachment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(page), Some(host)) = (&self.page, &self.host) else {
            return;
        };
        let attach = self.should_attach(window, cx);
        if attach != page.is_attached() {
            if attach {
                page.attach(host);
            } else {
                page.detach();
            }
        }
        // Notifications would hide behind the page in the top-right corner.
        let placement = if attach {
            Anchor::BottomLeft
        } else {
            Anchor::TopRight
        };
        if cx.theme().notification.placement != placement {
            Theme::global_mut(cx).notification.placement = placement;
        }
    }

    fn page(&mut self) -> Option<Rc<WebView>> {
        if self.page.is_none() {
            self.page = Some(Rc::new(WebView::new(
                self.events.clone(),
                &webview::configuration(),
            )));
        }
        self.page.clone()
    }

    /// Load `url`.
    pub fn open(&mut self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(page) = self.page() {
            page.load(url);
            self.state.url = url.to_string();
            self.address.update(cx, |address, cx| {
                address.set_value(url.to_string(), window, cx)
            });
            self.sync_attachment(window, cx);
            page.focus();
        }
        cx.notify();
    }

    /// The page, for agent tools.
    pub fn web_view(&self) -> Option<Rc<WebView>> {
        self.page.clone()
    }

    pub fn page_state(&self) -> &PageState {
        &self.state
    }

    pub fn is_on_screen(&self) -> bool {
        self.page.as_ref().is_some_and(|page| page.is_attached())
    }

    fn handle_event(&mut self, event: WebEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            WebEvent::Changed => {
                if let Some(page) = &self.page {
                    self.state = page.state();
                    let editing = self.address.read(cx).focus_handle(cx).is_focused(window);
                    if !editing {
                        let url = self.state.url.clone();
                        self.address
                            .update(cx, |address, cx| address.set_value(url, window, cx));
                    }
                }
                cx.notify();
            }
            // Keys now go to the page: release GPUI focus so shortcuts bound
            // to the composer don't take them.
            WebEvent::Focused => window.blur(cx),
            WebEvent::OpenExternally(url) => cx.open_url(&url),
            WebEvent::ProcessTerminated => {
                if let Some(page) = &self.page {
                    page.reload();
                }
            }
        }
    }

    fn toolbar(&self, cx: &Context<Self>) -> AnyElement {
        let state = &self.state;
        let has_page = self.page.is_some();
        let url = state.url.clone();
        h_flex()
            .h(px(36.))
            .px_2()
            .gap_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("browser-back")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ArrowLeft)
                    .tooltip("Back")
                    .disabled(!state.can_go_back)
                    .on_click(cx.listener(|this, _, _, _| {
                        if let Some(page) = &this.page {
                            page.go_back();
                        }
                    })),
            )
            .child(
                Button::new("browser-forward")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ArrowRight)
                    .tooltip("Forward")
                    .disabled(!state.can_go_forward)
                    .on_click(cx.listener(|this, _, _, _| {
                        if let Some(page) = &this.page {
                            page.go_forward();
                        }
                    })),
            )
            .child(
                Button::new("browser-reload")
                    .ghost()
                    .xsmall()
                    .icon(if state.loading {
                        IconName::X
                    } else {
                        IconName::RotateCw
                    })
                    .tooltip(if state.loading { "Stop" } else { "Reload" })
                    .disabled(!has_page)
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(page) = &this.page {
                            if this.state.loading {
                                page.stop();
                            } else {
                                page.reload();
                            }
                        }
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.address).small()),
            )
            .child(
                Button::new("browser-external")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ExternalLink)
                    .tooltip("Open in your browser")
                    .disabled(url.is_empty())
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            )
            .into_any_element()
    }

    fn start_page(&self, cx: &Context<Self>) -> AnyElement {
        let servers = self.servers.iter().map(|server| {
            let url = format!("http://localhost:{}", server.port);
            let open = url.clone();
            Button::new(SharedString::from(format!(
                "browser-server-{}",
                server.port
            )))
            .small()
            .icon(IconName::Globe)
            .label(format!("{url}  ·  {}", server.command))
            .on_click(cx.listener(move |this, _, window, cx| this.open(&open, window, cx)))
        });
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_3()
            .p_6()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .text_center()
                    .child("Open the app you're working on: type an address above, or pick a server running in this folder."),
            )
            .children(servers)
            .when(self.servers.is_empty(), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("No development servers found here yet — start one in the terminal."),
                )
            })
            .child(
                Button::new("browser-rescan")
                    .ghost()
                    .xsmall()
                    .icon(IconName::RefreshCw)
                    .label("Look again")
                    .on_click(cx.listener(|this, _, _, cx| this.scan_servers(cx))),
            )
            .into_any_element()
    }
}

impl Render for BrowserView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let progress = self
            .state
            .loading
            .then_some(self.state.progress.clamp(0.05, 1.0));
        let body: AnyElement = match (&self.page, &self.host) {
            (Some(page), Some(host)) => {
                let (page, host) = (page.clone(), host.clone());
                // The page goes where GPUI lays out this empty area.
                canvas(
                    move |bounds, _, _| page.set_frame(&host, bounds),
                    |_, _, _, _| {},
                )
                .size_full()
                .into_any_element()
            }
            (_, None) => div()
                .p_4()
                .text_sm()
                .child("The browser isn't available in this window.")
                .into_any_element(),
            (None, _) => self.start_page(cx),
        };
        v_flex()
            .size_full()
            .child(self.toolbar(cx))
            .child(
                div()
                    .h(px(2.))
                    .w_full()
                    .when_some(progress, |this, progress| {
                        this.child(
                            div()
                                .h_full()
                                .w(relative(progress as f32))
                                .bg(cx.theme().primary),
                        )
                    }),
            )
            .child(div().flex_1().min_h_0().child(body))
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_address;

    #[test]
    fn turns_input_into_addresses() {
        assert_eq!(
            normalize_address("localhost:5173").as_deref(),
            Some("http://localhost:5173")
        );
        assert_eq!(
            normalize_address("shop.test/cart").as_deref(),
            Some("http://shop.test/cart")
        );
        assert_eq!(
            normalize_address("example.com").as_deref(),
            Some("https://example.com")
        );
        assert_eq!(
            normalize_address("https://x.y/z").as_deref(),
            Some("https://x.y/z")
        );
        assert_eq!(
            normalize_address("rust gpui").as_deref(),
            Some("https://duckduckgo.com/?q=rust+gpui")
        );
        assert_eq!(normalize_address("  "), None);
    }
}
