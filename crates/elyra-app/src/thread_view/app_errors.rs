//! Errors from the app the thread is building: its server's log and the
//! pages in the thread's browser, offered to the agent or the author.

use super::*;

impl ThreadView {
    /// Show the thread's browser errors above the composer.
    pub fn set_browser(
        &mut self,
        browser: Entity<crate::browser_view::BrowserView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.browser.as_ref() == Some(&browser) {
            return;
        }
        self._subscriptions
            .push(cx.observe(&browser, |_, _, cx| cx.notify()));
        self._subscriptions.push(cx.subscribe_in(
            &browser,
            window,
            |this, _, event: &crate::browser_view::BrowserEvent, window, cx| {
                let crate::browser_view::BrowserEvent::ElementPicked {
                    title,
                    details,
                    picture,
                } = event;
                if let Some(picture) = picture {
                    this.attachments
                        .push(Attachment::image("image/jpeg", picture));
                }
                this.attachments.push(Attachment::Context {
                    title: title.clone(),
                    content: details.clone(),
                    replay: None,
                });
                this.focus_composer(window, cx);
                cx.notify();
            },
        ));
        self.browser = Some(browser);
        cx.notify();
    }

    /// Open a Félagi issue (type exception) for the newest server error, with
    /// Grove's explanation; link the thread to it when it has no issue yet.
    pub(super) fn file_server_error(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(error) = self.server_errors.pop() else {
            return;
        };
        let Some(app) = crate::preferences::app_state(cx) else {
            return;
        };
        let store = &app.read(cx).store;
        let Some(connection) = crate::felagi::connection(store) else {
            return;
        };
        let Some(client) = crate::felagi::Client::for_connection(&connection) else {
            window.push_notification("The Félagi token isn't in the Keychain.", cx);
            return;
        };
        let session = self.session.read(cx);
        let project = crate::felagi::linked_project(store, session.project.id);
        let link = session.thread.felagi_issue.is_none();
        cx.notify();
        let job = cx.background_executor().spawn(async move {
            let explanation = crate::grove::explain(error.id)
                .unwrap_or_else(|err| format!("{}\n({err:#})", error.line()));
            let escaped = explanation
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            let mut fields = serde_json::json!({
                "title": format!("Server error: {}", error.line()),
                "description": format!(
                    "<p>Recorded by Elyra Grove (request {}) and filed from Elyra Workspace.</p><pre>{escaped}</pre>",
                    error.id
                ),
                "type": "exception",
                "status": "todo",
                "priority": "high",
            });
            if let Some(project) = project {
                fields["project_id"] = project.into();
            }
            client.create_issue(fields)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = job.await;
            let _ = this.update_in(cx, |this, window, cx| match result {
                Ok(issue) => {
                    let id = issue.id.clone();
                    this.session.update(cx, |session, cx| {
                        session.notice(
                            format!("Filed {id} in Félagi: {}.", issue.title),
                            false,
                            cx,
                        );
                        if link {
                            session.link_felagi(id.clone(), cx);
                        }
                    });
                    if link {
                        let session = this.session.clone();
                        this.felagi = Some(
                            cx.new(|cx| crate::felagi_report::FelagiLink::new(session, id, cx)),
                        );
                    }
                    cx.notify();
                }
                Err(err) => {
                    window.push_notification(format!("Could not file it in Félagi: {err:#}"), cx)
                }
            });
        })
        .detach();
    }

    /// Errors the app logged, with the instruction behind the failing line.
    pub(super) fn saw_app_errors(
        &mut self,
        logged: Vec<(
            crate::app_errors::AppError,
            Option<(std::path::PathBuf, crate::app_errors::Culprit)>,
        )>,
        cx: &mut Context<Self>,
    ) {
        if logged.is_empty() {
            return;
        }
        let Some(app) = self.session.read(cx).app_entity() else {
            return;
        };
        let state = app.read(cx);
        let dir = self.session.read(cx).working_dir();
        for (error, culprit) in logged.into_iter().rev() {
            let (root, culprit) = match culprit {
                Some((root, culprit)) => (root, Some(culprit)),
                None => (dir.clone(), None),
            };
            self.app_errors
                .push(crate::app_errors::trace(state, &root, error, culprit));
        }
        let keep = self.app_errors.len().saturating_sub(MAX_SERVER_ERRORS);
        self.app_errors.drain(..keep);
        cx.notify();
    }

    /// Ask the author of lines this thread's turn took out whether the new
    /// code still does what it was asked to do.
    pub fn ask_rewrite_author(
        &mut self,
        item: ItemId,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self.session.read(cx).rewrite_request(item, index) else {
            window.push_notification("That instruction's lines can't be traced any more.", cx);
            return;
        };
        let Some(app) = self.session.read(cx).app_entity() else {
            return;
        };
        let session = self.session.clone();
        cx.spawn_in(window, async move |_, cx| {
            let result = crate::ask_line::ask(&app, request, cx).await;
            let _ = cx.update(|window, cx| match result {
                Ok(chat) => {
                    let chat = chat.read(cx).thread.id;
                    session.update(cx, |session, cx| {
                        session.set_rewrite_asked(item, index, chat, cx)
                    });
                    if let Some(hub) = cx.try_global::<crate::browser_tools::BrowserHub>() {
                        let workspace = hub.workspace.clone();
                        let _ =
                            workspace.update(cx, |this, cx| this.show_side_chat(chat, window, cx));
                    }
                }
                Err(err) => window.push_notification(format!("Could not ask: {err:#}"), cx),
            });
        })
        .detach();
    }

    /// Ask the agent that wrote the line the newest app error failed on.
    pub(super) fn ask_about_app_error(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(traced) = self.app_errors.last().cloned() else {
            return;
        };
        let (Some(culprit), Some(app)) =
            (traced.culprit.clone(), self.session.read(cx).app_entity())
        else {
            return;
        };
        let Ok(root) = elyra_git::repo_root(&self.session.read(cx).working_dir()) else {
            return;
        };
        let request = crate::ask_line::Request {
            parent: Some(self.session.read(cx).thread.id),
            root,
            path: culprit.path.clone(),
            line: Some(culprit.line),
            text: culprit.text.clone(),
            question: format!(
                "The app now fails on this line: {} ({}). What was the line meant to handle, and which assumption of yours doesn't hold? Say how you would fix it.",
                traced.error.message, traced.error.level
            ),
            ..Default::default()
        };
        self.app_errors.pop();
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = crate::ask_line::ask(&app, request, cx).await;
            let _ = this.update_in(cx, |_, window, cx| match result {
                Ok(chat) => {
                    let id = chat.read(cx).thread.id;
                    if let Some(hub) = cx.try_global::<crate::browser_tools::BrowserHub>() {
                        let workspace = hub.workspace.clone();
                        let _ =
                            workspace.update(cx, |this, cx| this.show_side_chat(id, window, cx));
                    }
                }
                Err(err) => window.push_notification(format!("Could not ask: {err:#}"), cx),
            });
        })
        .detach();
    }

    /// New server errors among Grove's latest requests for the app.
    pub(super) fn saw_requests(
        &mut self,
        requests: Vec<crate::grove::Request>,
        cx: &mut Context<Self>,
    ) {
        let (seen, fresh) = crate::grove::new_server_errors(self.server_seen, requests);
        self.server_seen = Some(seen);
        if !fresh.is_empty() {
            self.server_errors.extend(fresh);
            cx.notify();
        }
    }

    pub(super) fn add_browser_errors(&mut self, cx: &mut Context<Self>) {
        if let Some(browser) = self.browser.clone()
            && let Some((title, content)) =
                browser.update(cx, |browser, cx| browser.take_errors(cx))
        {
            self.attachments.push(Attachment::Context {
                title,
                content,
                replay: None,
            });
        }
        for traced in std::mem::take(&mut self.app_errors) {
            self.attachments.push(Attachment::Context {
                title: format!("App error: {}", traced.error.message),
                content: crate::app_errors::describe(&traced),
                replay: None,
            });
        }
        // Server errors come with Grove's explanation: the request, its SQL
        // and mail, and the error log.
        let errors: Vec<_> = std::mem::take(&mut self.server_errors)
            .into_iter()
            .rev()
            .take(MAX_SERVER_ERRORS)
            .collect();
        if !errors.is_empty() {
            let job = cx.background_executor().spawn(async move {
                errors
                    .into_iter()
                    .map(|error| {
                        let content = crate::grove::explain(error.id)
                            .unwrap_or_else(|err| format!("{}\n({err:#})", error.line()));
                        (
                            format!("Server error {} (Grove request {})", error.line(), error.id),
                            content,
                            (error.id, error.line()),
                        )
                    })
                    .collect::<Vec<_>>()
            });
            cx.spawn(async move |this, cx| {
                let explained = job.await;
                let _ = this.update(cx, |this, cx| {
                    for (title, content, request) in explained {
                        this.attachments.push(Attachment::Context {
                            title,
                            content,
                            replay: Some(request),
                        });
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        cx.notify();
    }

    pub(super) fn render_page_errors(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let count = self
            .browser
            .as_ref()
            .map_or(0, |browser| browser.read(cx).new_errors().len());
        let mut parts = Vec::new();
        if count > 0 {
            parts.push(format!(
                "{count} new error{} in the browser",
                if count == 1 { "" } else { "s" }
            ));
        }
        match self.server_errors.as_slice() {
            [] => {}
            [one] => parts.push(format!("server error: {}", one.line())),
            many => parts.push(format!("{} server errors", many.len())),
        }
        match self.app_errors.as_slice() {
            [] => {}
            [one] => parts.push(crate::app_errors::summary(one)),
            many => parts.push(format!(
                "{} app errors, newest: {}",
                many.len(),
                crate::app_errors::summary(&many[many.len() - 1])
            )),
        }
        let can_ask = self
            .app_errors
            .last()
            .is_some_and(|t| t.written_for.is_some());
        if parts.is_empty() {
            return None;
        }
        Some(
            h_flex()
                .gap_2()
                .px_2()
                .py_1()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().danger.opacity(0.4))
                .bg(cx.theme().danger.opacity(0.08))
                .text_xs()
                .child(
                    Icon::new(IconName::Bug)
                        .xsmall()
                        .text_color(cx.theme().danger),
                )
                .child(div().flex_1().min_w_0().whitespace_normal().child(parts.join(" · ")))
                .when(can_ask, |this| {
                    this.child(
                        Button::new("ask-app-error")
                            .xsmall()
                            .ghost()
                            .icon(IconName::MessagesSquare)
                            .label("Ask why")
                            .tooltip("Ask the agent that wrote the failing line, as it was when it wrote it")
                            .on_click(cx.listener(|this, _, window, cx| this.ask_about_app_error(window, cx))),
                    )
                })
                .when(!self.server_errors.is_empty() && felagi_can_write(cx), |this| {
                    this.child(
                        Button::new("file-server-error")
                            .xsmall()
                            .ghost()
                            .label("File in Félagi")
                            .tooltip("Open a Félagi issue for the newest server error, with Grove's explanation")
                            .on_click(cx.listener(|this, _, window, cx| this.file_server_error(window, cx))),
                    )
                })
                .child(
                    Button::new("add-browser-errors")
                        .xsmall()
                        .outline()
                        .label("Add to message")
                        .on_click(cx.listener(|this, _, _, cx| this.add_browser_errors(cx))),
                )
                .child(
                    Button::new("dismiss-browser-errors")
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .tooltip("Dismiss")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(browser) = &this.browser {
                                browser.update(cx, |browser, cx| browser.dismiss_errors(cx));
                            }
                            this.server_errors.clear();
                            this.app_errors.clear();
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }
}
