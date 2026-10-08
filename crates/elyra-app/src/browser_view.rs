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
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, Theme, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::Value;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

/// How often to check whether a dialog or sheet now covers the page.
const COVER_CHECK: Duration = Duration::from_millis(150);

/// How often to look for new errors on a local page.
const ERROR_CHECK: Duration = Duration::from_secs(2);

/// How often to look whether the user picked an element.
const PICK_CHECK: Duration = Duration::from_millis(150);

/// Room around a picked element in its picture.
const PICK_MARGIN: f64 = 16.;

pub enum BrowserEvent {
    /// The user picked an element on the page for the next message.
    ElementPicked {
        title: String,
        details: String,
        picture: Option<Vec<u8>>,
    },
}

/// A picked element (see element-pick.js) as a title and text for the agent.
pub fn element_details(picked: &Value) -> (String, String) {
    let text = |key: &str| picked[key].as_str().unwrap_or("").to_string();
    let number = |value: &Value| value.as_f64().unwrap_or(0.).round() as i64;
    let selector = text("selector");
    let title = format!("Element {selector} on {}", text("url"));
    let b = &picked["box"];
    let v = &picked["viewport"];
    let mut out = format!(
        "Selector: {selector}\nBox: x {}, y {}, {}×{} px (viewport {}×{})\n",
        number(&b["x"]),
        number(&b["y"]),
        number(&b["width"]),
        number(&b["height"]),
        number(&v["width"]),
        number(&v["height"]),
    );
    let inner = text("text");
    if !inner.is_empty() {
        out.push_str(&format!("Text: {inner}\n"));
    }
    if let Some(styles) = picked["styles"].as_object() {
        out.push_str("Computed styles:\n");
        for (name, value) in styles {
            let value = value.as_str().unwrap_or("");
            if !value.is_empty() {
                out.push_str(&format!("  {name}: {value}\n"));
            }
        }
    }
    out.push_str(&format!("HTML:\n{}", text("html")));
    (title, out)
}

/// An error on the page: something written with console.error or thrown and
/// not caught, or a request that failed.
#[derive(Clone, Debug, PartialEq)]
pub struct PageError {
    /// Tells the same error apart from a new one across checks.
    key: String,
    pub line: String,
}

/// The errors in what the page's watcher captured (see agent-capture.js).
/// Requests still under way are left out until they finish.
pub fn page_errors(captured: &Value) -> Vec<PageError> {
    let mut errors = Vec::new();
    for entry in captured["console"].as_array().into_iter().flatten() {
        if entry["level"] != "error" {
            continue;
        }
        let text = entry["text"].as_str().unwrap_or("");
        let short: String = text.chars().take(600).collect();
        errors.push(PageError {
            key: format!(
                "c:{}:{}",
                entry["time"],
                text.chars().take(80).collect::<String>()
            ),
            line: format!("console.error: {short}"),
        });
    }
    for entry in captured["network"].as_array().into_iter().flatten() {
        let status = entry["status"].as_u64();
        let error = entry["error"].as_str();
        let failed = error.is_some() || status.is_some_and(|s| s >= 400);
        if !failed {
            continue;
        }
        let url = entry["url"].as_str().unwrap_or("");
        let outcome = match (status, error) {
            (Some(status), _) if status > 0 => status.to_string(),
            (_, Some(error)) => error.to_string(),
            _ => "failed".into(),
        };
        let mut line = format!(
            "{} {url} → {outcome}",
            entry["method"].as_str().unwrap_or("GET")
        );
        if let Some(body) = entry["body"].as_str().filter(|b| !b.trim().is_empty()) {
            let body: String = body.chars().take(300).collect();
            line.push_str(&format!(" ({})", body.trim().replace('\n', " ")));
        }
        errors.push(PageError {
            key: format!("n:{}:{url}", entry["time"]),
            line,
        });
    }
    errors
}

pub struct BrowserView {
    host: Option<Rc<NativeHost>>,
    page: Option<Rc<WebView>>,
    events: async_channel::Sender<WebEvent>,
    address: Entity<InputState>,
    state: PageState,
    cwd: PathBuf,
    servers: Vec<Server>,
    /// The app Grove serves for this folder.
    grove: Option<crate::grove::Site>,
    /// The tab is on screen (set by the workspace on every render).
    shown: bool,
    /// The workspace draws something over the page (palette, About).
    covered: bool,
    /// Errors on the page the user hasn't added to a message or dismissed.
    errors: Vec<PageError>,
    /// Errors already shown and handled, so they don't come back.
    seen: HashSet<String>,
    /// "Pick element" is on: the next click on the page picks.
    picking: bool,
    /// The thread's agent may click and type on the page (until Take over).
    agent_control: bool,
    /// What agents did on the page since it was last opened, to save as a
    /// journey.
    recording: Vec<crate::journeys::Step>,
    _pick: Option<Task<()>>,
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
        // Look for new errors on local pages, for the composer's error chip.
        let errors = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(ERROR_CHECK).await;
                let (tx, rx) = async_channel::bounded::<Option<String>>(1);
                let asked = this.update(cx, |this, _| {
                    let page = this.page.clone()?;
                    webview::is_local_address(&this.state.url).then(|| {
                        page.agent_captured(move |json| {
                            tx.try_send(json).ok();
                        })
                    })
                });
                match asked {
                    Err(_) => break,
                    Ok(None) => continue,
                    Ok(Some(())) => {}
                }
                let captured = rx
                    .recv()
                    .await
                    .ok()
                    .flatten()
                    .and_then(|json| serde_json::from_str::<Value>(&json).ok());
                if let Some(captured) = captured
                    && this
                        .update(cx, |this, cx| this.update_errors(&captured, cx))
                        .is_err()
                {
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
            grove: None,
            shown: false,
            covered: false,
            errors: Vec::new(),
            seen: HashSet::new(),
            picking: false,
            agent_control: false,
            recording: Vec::new(),
            _pick: None,
            _tasks: vec![drain, cover, errors],
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
            .spawn(async move { (local_servers(&root), crate::grove::app_for(&root)) });
        cx.spawn(async move |this, cx| {
            let (servers, grove) = job.await;
            let _ = this.update(cx, |this, cx| {
                this.servers = servers;
                this.grove = grove;
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

    /// Start or stop "Pick element".
    pub fn toggle_pick(&mut self, cx: &mut Context<Self>) {
        let Some(page) = self.page.clone() else {
            return;
        };
        if self.picking {
            page.pick("stop()", |_| {});
            self.stop_picking(cx);
            return;
        }
        self.picking = true;
        page.pick("start()", |_| {});
        page.focus();
        self._pick = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(PICK_CHECK).await;
                let (tx, rx) = async_channel::bounded::<Option<String>>(1);
                let asked = this.update(cx, |this, _| {
                    let page = this.page.clone().filter(|_| this.picking)?;
                    page.pick("take()", move |json| {
                        tx.try_send(json).ok();
                    });
                    Some(())
                });
                if !matches!(asked, Ok(Some(()))) {
                    break;
                }
                let json = rx.recv().await.ok().flatten().unwrap_or_default();
                let Ok(picked) = serde_json::from_str::<Value>(&json) else {
                    continue;
                };
                let _ = this.update(cx, |this, cx| this.picked(picked, cx));
                break;
            }
        }));
        cx.notify();
    }

    fn stop_picking(&mut self, cx: &mut Context<Self>) {
        self.picking = false;
        self._pick = None;
        cx.notify();
    }

    /// The user clicked an element (or pressed Escape): take a picture of it
    /// and hand both to the thread.
    fn picked(&mut self, picked: Value, cx: &mut Context<Self>) {
        self.picking = false;
        cx.notify();
        if picked["cancelled"].as_bool() == Some(true) {
            return;
        }
        let (title, details) = element_details(&picked);
        let area = {
            let b = &picked["box"];
            let n = |key: &str| b[key].as_f64().unwrap_or(0.);
            (
                n("x") - PICK_MARGIN,
                n("y") - PICK_MARGIN,
                n("width") + 2. * PICK_MARGIN,
                n("height") + 2. * PICK_MARGIN,
            )
        };
        let Some(page) = self.page.clone() else {
            return;
        };
        let (tx, rx) = async_channel::bounded::<Option<Vec<u8>>>(1);
        page.snapshot_area(area, move |jpeg| {
            tx.try_send(jpeg).ok();
        });
        cx.spawn(async move |this, cx| {
            let picture = rx.recv().await.ok().flatten();
            let _ = this.update(cx, |_, cx| {
                cx.emit(BrowserEvent::ElementPicked {
                    title,
                    details,
                    picture,
                })
            });
        })
        .detach();
    }

    fn update_errors(&mut self, captured: &Value, cx: &mut Context<Self>) {
        let all = page_errors(captured);
        // Forget errors the page no longer has (it reloaded or dropped old entries).
        let current: HashSet<&str> = all.iter().map(|e| e.key.as_str()).collect();
        self.seen.retain(|key| current.contains(key.as_str()));
        let fresh: Vec<PageError> = all
            .into_iter()
            .filter(|e| !self.seen.contains(&e.key))
            .collect();
        if fresh != self.errors {
            self.errors = fresh;
            cx.notify();
        }
    }

    /// Errors on the page not yet added to a message or dismissed.
    pub fn new_errors(&self) -> &[PageError] {
        &self.errors
    }

    fn mark_seen(&mut self, cx: &mut Context<Self>) {
        self.seen
            .extend(self.errors.drain(..).map(|error| error.key));
        cx.notify();
    }

    /// The new errors as text for a message, and stop offering them.
    pub fn take_errors(&mut self, cx: &mut Context<Self>) -> Option<(String, String)> {
        if self.errors.is_empty() {
            return None;
        }
        let count = self.errors.len();
        let title = format!(
            "{count} error{} in the browser at {}",
            if count == 1 { "" } else { "s" },
            self.state.url
        );
        let content = self
            .errors
            .iter()
            .map(|e| e.line.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        self.mark_seen(cx);
        Some((title, content))
    }

    pub fn dismiss_errors(&mut self, cx: &mut Context<Self>) {
        self.mark_seen(cx);
    }

    /// The page, for agent tools.
    pub fn web_view(&self) -> Option<Rc<WebView>> {
        self.page.clone()
    }

    pub fn page_state(&self) -> &PageState {
        &self.state
    }

    /// Note a step an agent took (an open starts the recording over).
    pub fn record(&mut self, step: crate::journeys::Step) {
        if matches!(step, crate::journeys::Step::Open(_)) {
            self.recording.clear();
        }
        self.recording.push(step);
    }

    pub fn recording(&self) -> Vec<crate::journeys::Step> {
        self.recording.clone()
    }

    pub fn agent_control(&self) -> bool {
        self.agent_control
    }

    pub fn set_agent_control(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.agent_control != on {
            self.agent_control = on;
            cx.notify();
        }
    }

    pub fn is_on_screen(&self) -> bool {
        self.page.as_ref().is_some_and(|page| page.is_attached())
    }

    fn handle_event(&mut self, event: WebEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            WebEvent::Changed => {
                if let Some(page) = &self.page {
                    let previous = self.state.url.clone();
                    self.state = page.state();
                    // A new page has no picker running.
                    if self.picking && (self.state.loading || self.state.url != previous) {
                        self.stop_picking(cx);
                    }
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
                Button::new("browser-pick")
                    .xsmall()
                    .when(self.picking, |this| this.primary())
                    .when(!self.picking, |this| this.ghost())
                    .icon(IconName::Target)
                    .tooltip(if self.picking {
                        "Click an element on the page (Esc to stop)"
                    } else {
                        "Pick an element to show the agent"
                    })
                    .disabled(!has_page)
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_pick(cx))),
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
            .children(self.grove.as_ref().map(|site| {
                let url = site.url();
                let open = url.clone();
                Button::new("browser-grove")
                    .small()
                    .primary()
                    .icon(IconName::Globe)
                    .label(format!("{}  ·  Grove", url.trim_end_matches('/')))
                    .on_click(cx.listener(move |this, _, window, cx| this.open(&open, window, cx)))
            }))
            .children(servers)
            .when(self.servers.is_empty() && self.grove.is_none(), |this| {
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

impl EventEmitter<BrowserEvent> for BrowserView {}

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
            .when(self.agent_control, |this| {
                this.child(
                    h_flex()
                        .w_full()
                        .px_3()
                        .py_1()
                        .gap_2()
                        .bg(cx.theme().warning.opacity(0.12))
                        .border_b_1()
                        .border_color(cx.theme().warning)
                        .text_xs()
                        .child(
                            Icon::new(IconName::MousePointerClick)
                                .xsmall()
                                .text_color(cx.theme().warning),
                        )
                        .child(
                            div()
                                .flex_1()
                                .child("The agent can click and type on pages in this browser."),
                        )
                        .child(
                            Button::new("agent-take-over")
                                .xsmall()
                                .label("Take over")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.set_agent_control(false, cx)),
                                ),
                        ),
                )
            })
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
    use super::{element_details, normalize_address, page_errors};
    use serde_json::json;

    #[test]
    fn describes_a_picked_element() {
        let picked = json!({
            "url": "http://localhost:5173/cart",
            "selector": "#buy",
            "text": "Add to cart",
            "html": "<button id=\"buy\">Add to cart</button>",
            "box": {"x": 60, "y": 230.4, "width": 108, "height": 38},
            "viewport": {"width": 520, "height": 760},
            "styles": {"display": "inline-block", "padding": "8px 16px", "transform": ""},
        });
        let (title, details) = element_details(&picked);
        assert_eq!(title, "Element #buy on http://localhost:5173/cart");
        assert!(details.starts_with(
            "Selector: #buy\nBox: x 60, y 230, 108×38 px (viewport 520×760)\nText: Add to cart\n"
        ));
        assert!(details.contains("  padding: 8px 16px\n"));
        assert!(!details.contains("transform"));
        assert!(details.ends_with("HTML:\n<button id=\"buy\">Add to cart</button>"));
    }

    #[test]
    fn finds_console_errors_and_failed_requests() {
        let captured = json!({
            "console": [
                {"level": "log", "time": 1, "text": "loaded"},
                {"level": "error", "time": 2, "text": "Uncaught TypeError: x is undefined"},
            ],
            "network": [
                {"method": "GET", "url": "http://localhost/ok", "time": 3, "status": 200},
                {"method": "POST", "url": "http://localhost/api", "time": 4, "status": 500, "body": "boom\n"},
                {"method": "GET", "url": "http://localhost/down", "time": 5, "error": "TypeError: Load failed"},
                {"method": "GET", "url": "http://localhost/pending", "time": 6},
            ],
        });
        let lines: Vec<String> = page_errors(&captured).into_iter().map(|e| e.line).collect();
        assert_eq!(
            lines,
            [
                "console.error: Uncaught TypeError: x is undefined",
                "POST http://localhost/api → 500 (boom)",
                "GET http://localhost/down → TypeError: Load failed",
            ]
        );
    }

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
