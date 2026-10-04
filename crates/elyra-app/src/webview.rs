//! Bridge between GPUI and WebKit's `WKWebView`.
//!
//! Adapted from Litr (© Wirelabs AS), used here under the MIT licence with its
//! owner's permission.
//!
//! GPUI can't host native views, so a web view is a sibling of GPUI's Metal
//! view inside the window's content view, always on top of GPUI content.
//! `WebView::set_frame` positions it over the area GPUI lays out for it, and
//! anything GPUI would draw over that area (dialogs, the palette) needs the web
//! view detached first.
//!
//! WebKit reports changes through KVO and its delegate protocols.
//! `ElyraWebView` is its own observer and delegate and forwards everything as a
//! `WebEvent` on a channel that the browser drains on GPUI's main thread.

use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::NonNull;

use async_channel::Sender;
use gpui_kit::{Bounds, Pixels, Window};
use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyObject, NSObject, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send, sel};
use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSImage, NSResponder, NSView};
use objc2_foundation::{
    NSData, NSError, NSKeyValueObservingOptions, NSNumber, NSObjectNSKeyValueObserverRegistration,
    NSObjectProtocol, NSPoint, NSRect, NSSize, NSString, NSURL, NSURLRequest,
};
use objc2_web_kit::{
    WKContentWorld, WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate,
    WKNavigationResponse, WKNavigationResponsePolicy, WKSnapshotConfiguration, WKUIDelegate,
    WKWebView, WKWebViewConfiguration, WKWindowFeatures,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

pub enum WebEvent {
    /// Some observed page property changed; re-read `WebView::state`.
    Changed,
    /// The user clicked into the page (the web view took keyboard focus).
    Focused,
    /// Something to open outside the in-app browser: a download, or a link
    /// for another app (`mailto:`, `tel:`).
    OpenExternally(String),
    /// The page's web content process died; reload to recover.
    ProcessTerminated,
}

/// KVO key paths that change what the toolbar shows.
const OBSERVED_KEYS: [&str; 6] = [
    "title",
    "URL",
    "loading",
    "estimatedProgress",
    "canGoBack",
    "canGoForward",
];

static KVO_CONTEXT: u8 = 0;

fn kvo_context() -> *mut c_void {
    &KVO_CONTEXT as *const u8 as *mut c_void
}

/// Addresses the web view shows itself; anything else belongs to another app.
pub fn is_web_address(url: &str) -> bool {
    let scheme = url.split(':').next().unwrap_or("").to_ascii_lowercase();
    matches!(
        scheme.as_str(),
        "http" | "https" | "about" | "blob" | "data" | "file"
    )
}

/// Pages served from this Mac (development servers). Only these are visible to
/// agents.
pub fn is_local_address(url: &str) -> bool {
    let Some(rest) = url.split("://").nth(1) else {
        return false;
    };
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = if authority.starts_with('[') {
        authority
            .split(']')
            .next()
            .map(|h| format!("{h}]"))
            .unwrap_or_default()
    } else {
        authority.split(':').next().unwrap_or("").to_string()
    };
    let host = host.to_ascii_lowercase();
    host == "localhost"
        || host == "127.0.0.1"
        || host == "[::1]"
        || host.ends_with(".localhost")
        || host.ends_with(".test")
        || host.ends_with(".local")
}

/// Whether the user asked for this navigation (a click or a key press), as
/// WebKit tracks it (`_isUserInitiated`, SPI), else whether it's a link click.
fn user_initiated(action: &WKNavigationAction) -> bool {
    let selector = sel!(_isUserInitiated);
    let responds: bool = unsafe { msg_send![action, respondsToSelector: selector] };
    if responds {
        return unsafe { msg_send![action, _isUserInitiated] };
    }
    (unsafe { action.navigationType() }) == objc2_web_kit::WKNavigationType::LinkActivated
}

/// Editing shortcuts for the page. GPUI handles key equivalents first; the
/// ones it leaves alone reach the page through these selectors.
fn editing_selector(event: &NSEvent) -> Option<Sel> {
    let flags = event.modifierFlags();
    if !flags.contains(NSEventModifierFlags::Command)
        || flags.intersects(NSEventModifierFlags::Control | NSEventModifierFlags::Option)
    {
        return None;
    }
    let key = event
        .charactersIgnoringModifiers()?
        .to_string()
        .to_lowercase();
    let shift = flags.contains(NSEventModifierFlags::Shift);
    Some(match (key.as_str(), shift) {
        ("c", false) => sel!(copy:),
        ("v", false) => sel!(paste:),
        ("x", false) => sel!(cut:),
        ("a", false) => sel!(selectAll:),
        ("z", false) => sel!(undo:),
        ("z", true) => sel!(redo:),
        _ => return None,
    })
}

pub struct Ivars {
    events: Sender<WebEvent>,
}

define_class!(
    // SAFETY: WKWebView supports subclassing, and `ElyraWebView` doesn't implement Drop.
    #[unsafe(super(WKWebView, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "ElyraWorkspaceWebView"]
    #[ivars = Ivars]
    pub struct ElyraWebView;

    impl ElyraWebView {
        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> bool {
            let accepted: bool = unsafe { msg_send![super(self), becomeFirstResponder] };
            if accepted {
                self.send(WebEvent::Focused);
            }
            accepted
        }

        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            let handled: bool = unsafe { msg_send![super(self), performKeyEquivalent: event] };
            handled || self.perform_editing_shortcut(event)
        }

        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value(
            &self,
            key_path: Option<&NSString>,
            object: Option<&AnyObject>,
            change: Option<&AnyObject>,
            context: *mut c_void,
        ) {
            if context == kvo_context() {
                self.send(WebEvent::Changed);
            } else {
                unsafe {
                    let _: () = msg_send![
                        super(self),
                        observeValueForKeyPath: key_path,
                        ofObject: object,
                        change: change,
                        context: context
                    ];
                }
            }
        }
    }

    unsafe impl NSObjectProtocol for ElyraWebView {}

    unsafe impl WKNavigationDelegate for ElyraWebView {
        #[unsafe(method(webViewWebContentProcessDidTerminate:))]
        fn web_content_process_did_terminate(&self, _web_view: &WKWebView) {
            self.send(WebEvent::ProcessTerminated);
        }

        #[unsafe(method(webView:decidePolicyForNavigationAction:preferences:decisionHandler:))]
        fn decide_policy_for_action(
            &self,
            _web_view: &WKWebView,
            action: &WKNavigationAction,
            preferences: &objc2_web_kit::WKWebpagePreferences,
            decision_handler: &block2::DynBlock<
                dyn Fn(WKNavigationActionPolicy, NonNull<objc2_web_kit::WKWebpagePreferences>),
            >,
        ) {
            let url = unsafe { action.request().URL() }
                .and_then(|url| url.absoluteString())
                .map(|url| url.to_string())
                .unwrap_or_default();
            // A phone number, an e-mail address: another app's. Only on a
            // click, so a page can't start apps by itself.
            let elsewhere = !url.is_empty() && !is_web_address(&url);
            let download = unsafe { action.shouldPerformDownload() };
            if elsewhere || download {
                if user_initiated(action) || download {
                    self.send(WebEvent::OpenExternally(url));
                }
                decision_handler.call((WKNavigationActionPolicy::Cancel, NonNull::from(preferences)));
                return;
            }
            decision_handler.call((WKNavigationActionPolicy::Allow, NonNull::from(preferences)));
        }

        // Responses WebKit can't show (archives, installers): the system
        // browser downloads them.
        #[unsafe(method(webView:decidePolicyForNavigationResponse:decisionHandler:))]
        fn decide_policy_for_response(
            &self,
            _web_view: &WKWebView,
            response: &WKNavigationResponse,
            decision_handler: &block2::DynBlock<dyn Fn(WKNavigationResponsePolicy)>,
        ) {
            let showable = unsafe { response.canShowMIMEType() };
            if !showable
                && let Some(url) = unsafe { response.response().URL() }.and_then(|url| url.absoluteString())
            {
                self.send(WebEvent::OpenExternally(url.to_string()));
                decision_handler.call((WKNavigationResponsePolicy::Cancel,));
                return;
            }
            decision_handler.call((WKNavigationResponsePolicy::Allow,));
        }
    }

    unsafe impl WKUIDelegate for ElyraWebView {
        // `target=_blank` and `window.open`: one page per thread, so the link
        // opens in place.
        #[unsafe(method_id(webView:createWebViewWithConfiguration:forNavigationAction:windowFeatures:))]
        fn create_web_view(
            &self,
            _web_view: &WKWebView,
            _configuration: &WKWebViewConfiguration,
            action: &WKNavigationAction,
            _features: &WKWindowFeatures,
        ) -> Option<Retained<WKWebView>> {
            let request = unsafe { action.request() };
            let url = request.URL()
                .and_then(|url| url.absoluteString())
                .map(|url| url.to_string())
                .unwrap_or_default();
            if !url.is_empty() && !is_web_address(&url) {
                if user_initiated(action) {
                    self.send(WebEvent::OpenExternally(url));
                }
            } else {
                unsafe { self.loadRequest(&request) };
            }
            None
        }
    }
);

impl ElyraWebView {
    /// ⌘C, ⌘V, ⌘X, ⌘A, ⌘Z and ⇧⌘Z for the page, when it has keyboard focus.
    fn perform_editing_shortcut(&self, event: &NSEvent) -> bool {
        let focused = self
            .window()
            .and_then(|window| window.firstResponder())
            .is_some_and(|responder| {
                let object: &AnyObject = &responder;
                object
                    .downcast_ref::<NSView>()
                    .is_some_and(|view| view.isDescendantOf(self))
            });
        let Some(action) = editing_selector(event).filter(|_| focused) else {
            return false;
        };
        let responds: bool = unsafe { msg_send![self, respondsToSelector: action] };
        if responds {
            let _: *mut AnyObject =
                unsafe { msg_send![self, performSelector: action, withObject: None::<&AnyObject>] };
        }
        responds
    }

    fn send(&self, event: WebEvent) {
        // Fails only once the browser is gone, when there is nobody left to tell.
        self.ivars().events.try_send(event).ok();
    }
}

/// The window's native views: GPUI's Metal view and the content view that holds
/// it and the web views.
pub struct NativeHost {
    content_view: Retained<NSView>,
    gpui_view: Retained<NSView>,
}

impl NativeHost {
    pub fn new(window: &Window) -> Option<Self> {
        let RawWindowHandle::AppKit(handle) = HasWindowHandle::window_handle(window).ok()?.as_raw()
        else {
            return None;
        };
        // SAFETY: GPUI hands out a valid NSView pointer for the lifetime of the
        // window, and we are on the main thread.
        let gpui_view: &NSView = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
        let content_view = unsafe { gpui_view.superview() }.unwrap_or_else(|| gpui_view.retain());
        Some(Self {
            content_view,
            gpui_view: gpui_view.retain(),
        })
    }

    /// Give keyboard focus back to GPUI (the address bar, shortcuts).
    pub fn focus_gpui(&self) {
        if let Some(window) = self.gpui_view.window() {
            window.makeFirstResponder(Some(&self.gpui_view));
        }
    }

    /// GPUI's top-left window coordinates to AppKit's view coordinates.
    fn native_frame(&self, bounds: Bounds<Pixels>) -> NSRect {
        let x = f64::from(bounds.origin.x);
        let top = f64::from(bounds.origin.y);
        let w = f64::from(bounds.size.width);
        let h = f64::from(bounds.size.height);
        let y = if self.content_view.isFlipped() {
            top
        } else {
            self.content_view.bounds().size.height - top - h
        };
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
    }
}

/// What agents read back from resources/agent-capture.js: the console and
/// network it kept, and every file the page loaded.
const AGENT_READ: &str = "window.__elyraAgent ? JSON.stringify({console: window.__elyraAgent.console, network: window.__elyraAgent.network, resources: performance.getEntriesByType('resource').slice(-200).map(e => ({url: e.name, type: e.initiatorType, ms: Math.round(e.duration), bytes: e.transferSize, status: e.responseStatus || undefined}))}) : null";

/// The configuration for the in-app browser: persistent website data (shared
/// with nothing else), and the console and network watcher for agents, which
/// only starts on pages served from this Mac.
pub fn configuration() -> Retained<WKWebViewConfiguration> {
    let mtm = MainThreadMarker::new().expect("main thread");
    unsafe {
        let config = WKWebViewConfiguration::new(mtm);
        let script = objc2_web_kit::WKUserScript::initWithSource_injectionTime_forMainFrameOnly_inContentWorld(
            objc2_web_kit::WKUserScript::alloc(mtm),
            &NSString::from_str(include_str!("../resources/agent-capture.js")),
            objc2_web_kit::WKUserScriptInjectionTime::AtDocumentStart,
            true,
            &WKContentWorld::pageWorld(mtm),
        );
        config.userContentController().addUserScript(&script);
        let preferences = config.preferences();
        let fullscreen = sel!(setElementFullscreenEnabled:);
        if preferences.respondsToSelector(fullscreen) {
            let _: () = msg_send![&*preferences, setElementFullscreenEnabled: true];
        }
        config
    }
}

pub struct WebView {
    view: Retained<ElyraWebView>,
    last_frame: Cell<Option<Bounds<Pixels>>>,
}

impl WebView {
    pub fn new(events: Sender<WebEvent>, configuration: &WKWebViewConfiguration) -> Self {
        let mtm = MainThreadMarker::new().expect("web views live on the main thread");
        let this = ElyraWebView::alloc(mtm).set_ivars(Ivars { events });
        let view: Retained<ElyraWebView> = unsafe {
            msg_send![super(this), initWithFrame: NSRect::ZERO, configuration: configuration]
        };
        unsafe {
            view.setNavigationDelegate(Some(ProtocolObject::from_ref(&*view)));
            view.setUIDelegate(Some(ProtocolObject::from_ref(&*view)));
            view.setAllowsBackForwardNavigationGestures(true);
            // Safari's Web Inspector can attach (Develop menu).
            view.setInspectable(true);
            for key in OBSERVED_KEYS {
                view.addObserver_forKeyPath_options_context(
                    &view,
                    &NSString::from_str(key),
                    NSKeyValueObservingOptions::empty(),
                    kvo_context(),
                );
            }
        }
        Self {
            view,
            last_frame: Cell::new(None),
        }
    }

    pub fn is_attached(&self) -> bool {
        unsafe { self.view.superview() }.is_some()
    }

    /// Put the web view in the window, over its last frame.
    pub fn attach(&self, host: &NativeHost) {
        if !self.is_attached() {
            host.content_view.addSubview(&self.view);
            if let Some(bounds) = self.last_frame.get() {
                self.view.setFrame(host.native_frame(bounds));
            }
        }
    }

    pub fn detach(&self) {
        self.view.removeFromSuperview();
    }

    /// Position the web view over `bounds`, in GPUI window coordinates.
    pub fn set_frame(&self, host: &NativeHost, bounds: Bounds<Pixels>) {
        if self.last_frame.get() == Some(bounds) {
            return;
        }
        self.last_frame.set(Some(bounds));
        self.view.setFrame(host.native_frame(bounds));
    }

    pub fn focus(&self) {
        if let Some(window) = self.view.window() {
            window.makeFirstResponder(Some(&self.view));
        }
    }

    pub fn load(&self, url: &str) {
        let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) else {
            return;
        };
        unsafe {
            self.view.loadRequest(&NSURLRequest::requestWithURL(&url));
        }
    }

    pub fn go_back(&self) {
        unsafe { self.view.goBack() };
    }

    pub fn go_forward(&self) {
        unsafe { self.view.goForward() };
    }

    pub fn reload(&self) {
        unsafe { self.view.reload() };
    }

    pub fn stop(&self) {
        unsafe { self.view.stopLoading() };
    }

    pub fn state(&self) -> PageState {
        unsafe {
            PageState {
                title: self.view.title().map(|t| t.to_string()).unwrap_or_default(),
                url: self
                    .view
                    .URL()
                    .and_then(|u| u.absoluteString())
                    .map(|s| s.to_string())
                    .unwrap_or_default(),
                loading: self.view.isLoading(),
                progress: self.view.estimatedProgress(),
                can_go_back: self.view.canGoBack(),
                can_go_forward: self.view.canGoForward(),
            }
        }
    }

    fn evaluate_isolated(&self, script: &str, done: impl Fn(Option<&AnyObject>) + 'static) {
        let mtm = MainThreadMarker::new().expect("main thread");
        let handler = block2::RcBlock::new(move |result: *mut AnyObject, _error: *mut NSError| {
            done(unsafe { result.as_ref() });
        });
        unsafe {
            let world = WKContentWorld::defaultClientWorld(mtm);
            self.view
                .evaluateJavaScript_inFrame_inContentWorld_completionHandler(
                    &NSString::from_str(script),
                    None,
                    &world,
                    Some(&handler),
                );
        }
    }

    /// For an agent: `call` on resources/agent-look.js (`snapshot()`,
    /// `query(selector, limit)`) in the isolated world; `done` gets its JSON.
    pub fn agent_look(&self, call: &str, done: impl Fn(Option<String>) + 'static) {
        let script = format!(
            "{}\nwindow.__elyraLook.{call}",
            include_str!("../resources/agent-look.js")
        );
        self.evaluate_isolated(&script, move |result| {
            done(
                result
                    .and_then(|value| value.downcast_ref::<NSString>())
                    .map(|value| value.to_string()),
            );
        });
    }

    /// "Pick element": `call` on resources/element-pick.js (`start()`,
    /// `stop()`, `take()`) in the isolated world; `done` gets its string.
    pub fn pick(&self, call: &str, done: impl Fn(Option<String>) + 'static) {
        let script = format!(
            "{}\nwindow.__elyraPick.{call}",
            include_str!("../resources/element-pick.js")
        );
        self.evaluate_isolated(&script, move |result| {
            done(
                result
                    .and_then(|value| value.downcast_ref::<NSString>())
                    .map(|value| value.to_string()),
            );
        });
    }

    /// A JPEG of part of the page (CSS pixels from the top left of the
    /// view), or `None` when it isn't on screen.
    pub fn snapshot_area(
        &self,
        (x, y, width, height): (f64, f64, f64, f64),
        done: impl Fn(Option<Vec<u8>>) + 'static,
    ) {
        let mtm = MainThreadMarker::new().expect("main thread");
        let bounds = self.view.bounds().size;
        if bounds.width < 1. || self.view.window().is_none() {
            done(None);
            return;
        }
        let (left, top) = (x.max(0.), y.max(0.));
        let right = (x + width).min(bounds.width);
        let bottom = (y + height).min(bounds.height);
        if right - left < 1. || bottom - top < 1. {
            done(None);
            return;
        }
        let handler = block2::RcBlock::new(move |image: *mut NSImage, _error: *mut NSError| {
            done(unsafe { image.as_ref() }.and_then(|image| encode_jpeg(image, 0.8)));
        });
        unsafe {
            let config = WKSnapshotConfiguration::new(mtm);
            config.setRect(NSRect::new(
                NSPoint::new(left, top),
                NSSize::new(right - left, bottom - top),
            ));
            self.view
                .takeSnapshotWithConfiguration_completionHandler(Some(&config), &handler);
        }
    }

    /// For an agent: what resources/agent-capture.js kept of the page's
    /// console and network, as JSON, or `None` if it isn't running.
    pub fn agent_captured(&self, done: impl Fn(Option<String>) + 'static) {
        let handler = block2::RcBlock::new(move |result: *mut AnyObject, _error: *mut NSError| {
            done(
                unsafe { result.as_ref() }
                    .and_then(|value| value.downcast_ref::<NSString>())
                    .map(|value| value.to_string()),
            );
        });
        unsafe {
            self.view.evaluateJavaScript_completionHandler(
                &NSString::from_str(AGENT_READ),
                Some(&handler),
            );
        }
    }

    /// For an agent: a JPEG of the page as it is on screen, or `None` when it
    /// isn't on screen.
    pub fn agent_snapshot(&self, done: impl Fn(Option<Vec<u8>>) + 'static) {
        let mtm = MainThreadMarker::new().expect("main thread");
        let width = self.view.bounds().size.width;
        if width < 1. || self.view.window().is_none() {
            done(None);
            return;
        }
        let handler = block2::RcBlock::new(move |image: *mut NSImage, _error: *mut NSError| {
            done(unsafe { image.as_ref() }.and_then(|image| encode_jpeg(image, 0.7)));
        });
        unsafe {
            let config = WKSnapshotConfiguration::new(mtm);
            config.setSnapshotWidth(Some(&NSNumber::new_f64(width)));
            self.view
                .takeSnapshotWithConfiguration_completionHandler(Some(&config), &handler);
        }
    }
}

impl Drop for WebView {
    fn drop(&mut self) {
        unsafe {
            for key in OBSERVED_KEYS {
                self.view.removeObserver_forKeyPath_context(
                    &self.view,
                    &NSString::from_str(key),
                    kvo_context(),
                );
            }
            self.view.setNavigationDelegate(None);
            self.view.setUIDelegate(None);
            self.view.stopLoading();
        }
        self.view.removeFromSuperview();
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PageState {
    pub title: String,
    pub url: String,
    pub loading: bool,
    pub progress: f64,
    pub can_go_back: bool,
    pub can_go_forward: bool,
}

/// Redraw `image` at 1x scale (points = pixels) and encode it as JPEG.
fn encode_jpeg(image: &NSImage, quality: f64) -> Option<Vec<u8>> {
    unsafe {
        let size: NSSize = msg_send![image, size];
        let (width, height) = (size.width.round() as isize, size.height.round() as isize);
        if width < 1 || height < 1 {
            return None;
        }
        let rep: Allocated<AnyObject> = msg_send![objc2::class!(NSBitmapImageRep), alloc];
        let color_space = NSString::from_str("NSDeviceRGBColorSpace");
        let rep: Option<Retained<AnyObject>> = msg_send![
            rep,
            initWithBitmapDataPlanes: std::ptr::null_mut::<*mut u8>(),
            pixelsWide: width,
            pixelsHigh: height,
            bitsPerSample: 8isize,
            samplesPerPixel: 4isize,
            hasAlpha: true,
            isPlanar: false,
            colorSpaceName: &*color_space,
            bytesPerRow: 0isize,
            bitsPerPixel: 0isize
        ];
        let rep = rep?;
        let context: Option<Retained<AnyObject>> =
            msg_send![objc2::class!(NSGraphicsContext), graphicsContextWithBitmapImageRep: &*rep];
        let context = context?;
        let _: () = msg_send![objc2::class!(NSGraphicsContext), saveGraphicsState];
        let _: () = msg_send![objc2::class!(NSGraphicsContext), setCurrentContext: &*context];
        let rect = NSRect::new(
            NSPoint::new(0., 0.),
            NSSize::new(width as f64, height as f64),
        );
        let _: () = msg_send![
            image,
            drawInRect: rect,
            fromRect: NSRect::ZERO,
            operation: 2usize, // NSCompositingOperationSourceOver
            fraction: 1f64
        ];
        let _: () = msg_send![objc2::class!(NSGraphicsContext), restoreGraphicsState];
        let key = NSString::from_str("NSImageCompressionFactor");
        let factor = NSNumber::new_f64(quality);
        let properties: Retained<AnyObject> = msg_send![
            objc2::class!(NSDictionary),
            dictionaryWithObject: &*factor,
            forKey: &*key
        ];
        // NSBitmapImageFileTypeJPEG = 3.
        let jpeg: Option<Retained<NSData>> =
            msg_send![&rep, representationUsingType: 3usize, properties: &*properties];
        Some(jpeg?.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::{is_local_address, is_web_address};

    #[test]
    fn classifies_addresses() {
        assert!(is_web_address("https://example.com"));
        assert!(!is_web_address("mailto:a@b.c"));
        assert!(is_local_address("http://localhost:5173/app"));
        assert!(is_local_address("http://127.0.0.1:8000"));
        assert!(is_local_address("https://shop.test/cart"));
        assert!(is_local_address("http://[::1]:3000/"));
        assert!(!is_local_address("https://github.com/"));
        assert!(!is_local_address("https://localhost.evil.com/"));
        assert!(!is_local_address("file:///etc/hosts"));
    }
}
