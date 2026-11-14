//! Each thread's browser, the sites it can open, and pictures of a page
//! before and after a turn.

use super::*;

/// How long a reloaded page gets to settle before its picture is taken.
pub(super) const PAGE_SETTLE: std::time::Duration = std::time::Duration::from_millis(2500);

/// Wait for a page picture and keep it under the thread's snapshot folder.
pub(super) async fn save_page_picture(
    id: ThreadId,
    rx: async_channel::Receiver<Option<Vec<u8>>>,
    cx: &mut AsyncApp,
) -> Option<String> {
    let jpeg = rx.recv().await.ok().flatten()?;
    cx.background_executor()
        .spawn(async move {
            let dir = elyra_core::paths::snapshots_dir().join(id.to_string());
            std::fs::create_dir_all(&dir).ok()?;
            let path = dir.join(format!("{}.jpg", uuid::Uuid::new_v4().simple()));
            std::fs::write(&path, jpeg).ok()?;
            Some(path.to_string_lossy().into_owned())
        })
        .await
}

impl Workspace {
    /// The thread's browser, created on first use.
    pub(crate) fn browser_view(
        &mut self,
        id: ThreadId,
        cwd: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<crate::browser_view::BrowserView> {
        if let Some(view) = self.browser_views.get(&id) {
            view.update(cx, |view, cx| view.set_cwd(cwd, cx));
            return view.clone();
        }
        let view = cx.new(|cx| crate::browser_view::BrowserView::new(cwd, window, cx));
        if let Some(thread) = self.thread_views.get(&id) {
            let browser = view.clone();
            thread.update(cx, |thread, cx| thread.set_browser(browser, window, cx));
        }
        self.browser_views.insert(id, view.clone());
        view
    }

    /// Open `url` in a thread's browser, and show it when that thread is in
    /// front.
    /// The thread's browser, made (blank) if it has none yet.
    pub(crate) fn browser_for(
        &mut self,
        id: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<crate::browser_view::BrowserView>> {
        let cwd = {
            let state = self.app.read(cx);
            let thread = state.thread(id)?;
            thread.working_dir(state.project(thread.project_id)?)
        };
        Some(self.browser_view(id, cwd, window, cx))
    }

    pub(crate) fn open_in_browser(
        &mut self,
        id: ThreadId,
        url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<crate::browser_view::BrowserView>> {
        let cwd = {
            let state = self.app.read(cx);
            let thread = state.thread(id)?;
            thread.working_dir(state.project(thread.project_id)?)
        };
        let view = self.browser_view(id, cwd, window, cx);
        if self.active == Some(id) {
            self.show_right_tab(RightTab::Browser, window, cx);
        }
        view.update(cx, |view, cx| view.open(url, window, cx));
        Some(view)
    }

    /// Tell every browser whether it's on screen and uncovered.
    pub(super) fn sync_browsers(
        &mut self,
        panel_open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let covered = self.palette.is_some() || self.about_open;
        let visible_tab = self.right_open
            && self.right_tab == RightTab::Browser
            && !panel_open
            && !self.terminal_full;
        for (id, view) in self.browser_views.clone() {
            let shown = visible_tab && self.active == Some(id);
            view.update(cx, |view, cx| view.set_shown(shown, covered, window, cx));
        }
    }

    pub(super) fn on_show_browser(
        &mut self,
        _: &ShowBrowser,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_right_tab(RightTab::Browser, window, cx);
    }

    // ---- fork, handoff, import -------------------------------------------

    /// Load a thread's own Grove site in its browser, without switching tabs.
    pub(super) fn open_site(
        &mut self,
        id: ThreadId,
        url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(cwd) = self.app.read(cx).thread(id).and_then(|thread| {
            let project = self.app.read(cx).project(thread.project_id)?;
            Some(thread.working_dir(project))
        }) else {
            return;
        };
        let browser = self.browser_view(id, cwd, window, cx);
        browser.update(cx, |browser, cx| browser.open(url, window, cx));
    }

    // ---- page pictures around turns ------------------------------------------

    /// Ask the thread's browser for a picture of its page, when it shows a
    /// local page on screen (WebKit only pictures what is on screen).
    pub(super) fn picture_page(
        &self,
        id: ThreadId,
        cx: &App,
    ) -> Option<(String, async_channel::Receiver<Option<Vec<u8>>>)> {
        let browser = self.browser_views.get(&id)?.read(cx);
        let url = browser.page_state().url.clone();
        if !browser.is_on_screen() || !crate::webview::is_local_address(&url) {
            return None;
        }
        let page = browser.web_view()?;
        let (tx, rx) = async_channel::bounded(1);
        page.agent_snapshot(move |jpeg| {
            tx.try_send(jpeg).ok();
        });
        Some((url, rx))
    }

    pub(super) fn picture_page_before(&mut self, id: ThreadId, cx: &mut Context<Self>) {
        self.page_before.remove(&id);
        let Some((url, rx)) = self.picture_page(id, cx) else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let Some(path) = save_page_picture(id, rx, cx).await else {
                return;
            };
            let _ = this.update(cx, |this, _| this.page_before.insert(id, (url, path)));
        })
        .detach();
    }

    /// After a turn: reload the page, give the dev server a moment, and keep
    /// a picture next to the one from before the turn.
    pub(super) fn picture_page_after(&mut self, id: ThreadId, cx: &mut Context<Self>) {
        let on_screen = self.browser_views.get(&id).is_some_and(|browser| {
            let browser = browser.read(cx);
            browser.is_on_screen() && crate::webview::is_local_address(&browser.page_state().url)
        });
        if !on_screen {
            self.page_before.remove(&id);
            return;
        }
        if let Some(page) = self.browser_views[&id].read(cx).web_view() {
            page.reload();
        }
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(PAGE_SETTLE).await;
            let Ok(Some((url, rx))) = this.update(cx, |this, cx| this.picture_page(id, cx)) else {
                return;
            };
            let Some(after) = save_page_picture(id, rx, cx).await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                let before = this
                    .page_before
                    .remove(&id)
                    .filter(|(before_url, _)| *before_url == url)
                    .map(|(_, path)| path);
                if let Some(session) = this.app.update(cx, |app, cx| app.session(id, cx)) {
                    session.update(cx, |session, cx| {
                        session.add_page_snapshot(url, before, after, cx)
                    });
                }
            });
        })
        .detach();
    }
}
