//! The composer: typing and its history, the popups for files, commands
//! and mentions, attachments, the agent suggested for a new thread, and sending.

use super::*;

impl ThreadView {
    pub fn focus_composer(&self, window: &mut Window, cx: &mut App) {
        self.composer
            .update(cx, |composer, cx| composer.focus(window, cx));
    }

    pub(super) fn composer_focused(&self, window: &Window, cx: &App) -> bool {
        self.composer.read(cx).focus_handle(cx).is_focused(window)
    }

    pub(super) fn set_composer(
        &mut self,
        text: &str,
        cursor: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.composer.update(cx, |composer, cx| {
            composer.set_value(text.to_string(), window, cx);
            let offset = cursor.unwrap_or(text.len()).min(text.len());
            let position = composer.text().offset_to_position(offset);
            composer.set_cursor_position(position, window, cx);
        });
        self.composer_changed(cx);
    }

    // ---- keyboard ---------------------------------------------------------

    pub(super) fn intercept(
        &mut self,
        event: &KeystrokeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.composer_focused(window, cx) {
            return;
        }
        let keystroke = &event.keystroke;
        let mods = keystroke.modifiers;
        let key = keystroke.key.as_str();
        if mods.platform && key == "v" && !mods.shift && self.paste_special(cx) {
            cx.stop_propagation();
            return;
        }
        let plain = !mods.platform && !mods.control && !mods.alt;
        if self.popup.is_some() && plain {
            let handled = match key {
                "up" => {
                    self.move_popup(-1, cx);
                    true
                }
                "down" => {
                    self.move_popup(1, cx);
                    true
                }
                "enter" | "tab" if !mods.shift => {
                    self.accept_popup(window, cx);
                    true
                }
                "escape" => {
                    self.popup = None;
                    cx.notify();
                    true
                }
                _ => false,
            };
            if handled {
                cx.stop_propagation();
            }
            return;
        }
        if (key == "up" || key == "down") && plain && !mods.shift {
            let value = self.composer.read(cx).value();
            if (value.is_empty() || self.history.is_some())
                && self.browse_history(key == "up", window, cx)
            {
                cx.stop_propagation();
            }
        }
    }

    pub(super) fn browse_history(
        &mut self,
        back: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let prompts: Vec<String> = self
            .session
            .read(cx)
            .items
            .iter()
            .rev()
            .filter_map(|item| match &item.content {
                ItemContent::User { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        if prompts.is_empty() {
            return false;
        }
        let next = match (self.history, back) {
            (None, true) => Some(0),
            (None, false) => return false,
            (Some(i), true) => Some((i + 1).min(prompts.len() - 1)),
            (Some(0), false) => None,
            (Some(i), false) => Some(i - 1),
        };
        let text = next.map(|i| prompts[i].clone()).unwrap_or_default();
        self.set_composer(&text, None, window, cx);
        self.history = next;
        true
    }

    /// Paste images and very long text as attachments.
    pub(super) fn paste_special(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        for entry in item.entries() {
            if let ClipboardEntry::Image(image) = entry {
                self.attachments.push(Attachment::image(
                    composer::mime_of(image.format),
                    &image.bytes,
                ));
                cx.notify();
                return true;
            }
        }
        if let Some(text) = item.text()
            && composer::is_large_paste(&text)
        {
            self.attachments.push(Attachment::pasted_text(text));
            cx.notify();
            return true;
        }
        false
    }

    // ---- composer popup -----------------------------------------------------

    pub(super) fn composer_changed(&mut self, cx: &mut Context<Self>) {
        let (value, cursor) = {
            let composer = self.composer.read(cx);
            (composer.value().to_string(), composer.cursor())
        };
        self.session
            .update(cx, |session, _| session.draft = value.clone());
        if !self.warmed_up && !value.is_empty() {
            self.warmed_up = true;
            self.session.update(cx, |session, cx| session.warm_up(cx));
        }
        self.refresh_popup(&value, cursor, cx);
        self.suggest_agent(value, cx);
    }

    /// While the first message is written: which agent did best where it
    /// points (after a pause in typing).
    pub(super) fn suggest_agent(&mut self, text: String, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        if session.has_started() || text.trim().chars().count() < 12 {
            self._route_task = None;
            if self.route.take().is_some() {
                cx.notify();
            }
            return;
        }
        let Some(app) = session.app_entity() else {
            return;
        };
        let (project, dir) = (session.project.id, session.project.path.clone());
        let records = crate::routing::cached(project);
        let threads = records
            .is_none()
            .then(|| crate::routing::threads(app.read(cx), project));
        self._route_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(500))
                .await;
            let message = text.clone();
            let pick = cx
                .background_executor()
                .spawn(async move {
                    let records = match records {
                        Some(records) => records,
                        None => {
                            let records = std::sync::Arc::new(
                                crate::routing::load(
                                    &elyra_core::paths::database_path(),
                                    project,
                                    &threads.unwrap_or_default(),
                                )
                                .unwrap_or_default(),
                            );
                            crate::routing::remember(project, records.clone());
                            records
                        }
                    };
                    let areas = crate::routing::areas_for(&dir, &message);
                    crate::routing::recommend(&records, &areas)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let current = {
                    let thread = &this.session.read(cx).thread;
                    (thread.provider, thread.model.clone())
                };
                this.route = pick
                    .filter(|pick| (pick.provider, pick.model.clone()) != current)
                    .map(|pick| (text, pick));
                cx.notify();
            });
        }));
    }

    /// Give the thread to the suggested agent.
    pub(super) fn take_route(&mut self, cx: &mut Context<Self>) {
        let Some((_, pick)) = self.route.take() else {
            return;
        };
        self.session.update(cx, |session, cx| {
            session.set_provider(pick.provider, cx);
            session.set_model(pick.model.clone(), cx);
        });
        cx.notify();
    }

    pub(super) fn render_route(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let (_, pick) = self.route.as_ref()?;
        let name = match &pick.model {
            Some(model) => format!("{} · {model}", pick.provider.label()),
            None => pick.provider.label().to_string(),
        };
        Some(
            h_flex()
                .gap_2()
                .px_2()
                .py_1()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().info.opacity(0.4))
                .bg(cx.theme().info.opacity(0.06))
                .text_xs()
                .child(
                    Icon::new(IconName::ChartColumn)
                        .xsmall()
                        .text_color(cx.theme().info),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .whitespace_normal()
                        .child(pick.reason.clone()),
                )
                .child(
                    Button::new("route-take")
                        .xsmall()
                        .outline()
                        .label(format!("Use {name}"))
                        .on_click(cx.listener(|this, _, _, cx| this.take_route(cx))),
                )
                .child(
                    Button::new("route-dismiss")
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .tooltip("Dismiss")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.route = None;
                            cx.notify();
                        })),
                )
                .into_any_element(),
        )
    }

    pub(super) fn refresh_popup(&mut self, value: &str, cursor: usize, cx: &mut Context<Self>) {
        let Some((kind, start, query)) = composer::token_at(value, cursor) else {
            if self.popup.take().is_some() {
                cx.notify();
            }
            return;
        };
        let items = match kind {
            PopupKind::Slash => composer::slash_items(&self.session.read(cx).commands, &query),
            PopupKind::Mention => {
                self.ensure_file_index(cx);
                let mut items = composer::agent_items(&self.session.read(cx).agents, &query);
                if let Some(index) = &self.file_index {
                    items.extend(composer::mention_items(index, &query));
                }
                items
            }
        };
        let selected = self
            .popup
            .as_ref()
            .filter(|p| p.kind == kind)
            .map(|p| p.selected.min(items.len().saturating_sub(1)))
            .unwrap_or(0);
        let show = !items.is_empty() || (kind == PopupKind::Mention && self.indexing);
        self.popup = show.then_some(Popup {
            kind,
            start,
            end: cursor,
            items,
            selected,
        });
        cx.notify();
    }

    pub(super) fn ensure_file_index(&mut self, cx: &mut Context<Self>) {
        if self.file_index.is_some() || self.indexing {
            return;
        }
        self.indexing = true;
        let root = self.session.read(cx).working_dir();
        let job = cx
            .background_executor()
            .spawn(async move { composer::index_files(&root) });
        cx.spawn(async move |this, cx| {
            let files = job.await;
            let _ = this.update(cx, |this, cx| {
                this.indexing = false;
                this.file_index = Some(Arc::new(files));
                let (value, cursor) = {
                    let composer = this.composer.read(cx);
                    (composer.value().to_string(), composer.cursor())
                };
                this.refresh_popup(&value, cursor, cx);
            });
        })
        .detach();
    }

    pub(super) fn move_popup(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(popup) = &mut self.popup {
            let len = popup.items.len() as isize;
            if len > 0 {
                popup.selected = (popup.selected as isize + delta).rem_euclid(len) as usize;
            }
        }
        cx.notify();
    }

    pub(super) fn accept_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(popup) = self.popup.take() else {
            return;
        };
        let Some(item) = popup.items.get(popup.selected) else {
            cx.notify();
            return;
        };
        let value = self.composer.read(cx).value().to_string();
        let (text, cursor) = composer::apply_insert(&value, popup.start, popup.end, &item.insert);
        self.set_composer(&text, Some(cursor), window, cx);
        self.popup = None;
        cx.notify();
    }

    pub(super) fn pick_popup_item(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(popup) = &mut self.popup {
            popup.selected = index;
        }
        self.accept_popup(window, cx);
    }

    // ---- attachments --------------------------------------------------------

    pub(super) fn attach_paths(
        &mut self,
        paths: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let root = self.session.read(cx).working_dir();
        let mut mentions = Vec::new();
        for path in paths {
            let image = composer::image_mime_for_path(path)
                .and_then(|mime| Some((mime, std::fs::read(path).ok()?)));
            match image {
                Some((mime, bytes)) => self.attachments.push(Attachment::image(mime, &bytes)),
                None => mentions.push(composer::mention_for(path, &root)),
            }
        }
        if !mentions.is_empty() {
            let value = self.composer.read(cx).value().to_string();
            let separator = if value.is_empty() || value.ends_with(' ') || value.ends_with('\n') {
                ""
            } else {
                " "
            };
            let text = format!("{value}{separator}{} ", mentions.join(" "));
            self.set_composer(&text, None, window, cx);
        }
        self.focus_composer(window, cx);
        cx.notify();
    }

    pub(super) fn pick_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: true,
            prompt: Some("Attach".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await {
                let _ =
                    this.update_in(cx, |this, window, cx| this.attach_paths(&paths, window, cx));
            }
        })
        .detach();
    }

    // ---- sending ------------------------------------------------------------

    pub(super) fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.popup.is_some() {
            self.accept_popup(window, cx);
            return;
        }
        let text = self.composer.read(cx).value().to_string();
        if text.trim().is_empty() && self.attachments.is_empty() {
            return;
        }
        if self.attachments.is_empty() && self.run_builtin(text.trim(), cx) {
            self.set_composer("", None, window, cx);
            return;
        }
        // Settings → Pick the agent from your stats: take the suggestion
        // worked out for this very message.
        if crate::preferences::Preferences::global(cx).auto_route
            && !self.session.read(cx).has_started()
            && self
                .route
                .as_ref()
                .is_some_and(|(for_text, _)| *for_text == text)
            && let Some((_, pick)) = self.route.clone()
        {
            self.take_route(cx);
            self.session.update(cx, |session, cx| {
                session.notice(
                    format!(
                        "Given to {} from your stats. {}",
                        pick.provider.label(),
                        pick.reason
                    ),
                    false,
                    cx,
                )
            });
        }
        // Failed requests sent along are replayed after the turn.
        let replays: Vec<(u64, String)> = self
            .attachments
            .iter()
            .filter_map(|attachment| match attachment {
                Attachment::Context {
                    replay: Some(request),
                    ..
                } => Some(request.clone()),
                _ => None,
            })
            .collect();
        // Errors sent along can be kept as a test once the turn is done.
        let errors: Vec<(String, String)> = self
            .attachments
            .iter()
            .filter_map(|attachment| match attachment {
                Attachment::Context { title, content, .. }
                    if title.starts_with("Server error") || title.starts_with("App error") =>
                {
                    Some((title.clone(), content.clone()))
                }
                _ => None,
            })
            .collect();
        if !errors.is_empty() {
            self.session
                .update(cx, |session, _| session.errors_in_turn = errors);
        }
        let prompt = composer::build_prompt(&text, std::mem::take(&mut self.attachments));
        if !replays.is_empty() {
            self.session
                .update(cx, |session, _| session.replay_after_turn.extend(replays));
        }
        self.set_composer("", None, window, cx);
        self.history = None;
        self.session
            .update(cx, |session, cx| session.submit(prompt, cx));
        // Back to the end, following what comes.
        self.list.set_follow_mode(FollowMode::Tail);
    }

    /// App-level slash commands. Returns true when handled.
    pub(super) fn run_builtin(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
        let Some(command) = text.strip_prefix('/') else {
            return false;
        };
        let (name, args) = command
            .split_once(char::is_whitespace)
            .unwrap_or((command, ""));
        let args = args.trim().to_string();
        let session = self.session.clone();
        match name {
            "clear" | "reset" | "new" => session.update(cx, |s, cx| s.clear_context(cx)),
            "compact" if args.is_empty() => session.update(cx, |s, cx| s.compact(cx)),
            "rename" if !args.is_empty() => session.update(cx, |s, cx| s.rename(args, cx)),
            "plan" if args.is_empty() => {
                session.update(cx, |s, cx| s.set_permission_mode(PermissionMode::Plan, cx))
            }
            "default" if args.is_empty() => {
                session.update(cx, |s, cx| s.set_permission_mode(PermissionMode::Ask, cx))
            }
            "status" => session.update(cx, |s, cx| {
                let text = composer::status_text(s);
                s.notice(text, false, cx)
            }),
            _ => return false,
        }
        true
    }

    /// Add a block of text (e.g. a review comment) to the composer.
    pub fn append_to_composer(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.composer.read(cx).value().to_string();
        let joined = if value.trim().is_empty() {
            text.to_string()
        } else {
            format!("{}\n{text}", value.trim_end())
        };
        self.set_composer(&joined, None, window, cx);
        self.focus_composer(window, cx);
    }

    pub fn edit_message(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.set_composer(&text, None, window, cx);
        self.focus_composer(window, cx);
    }

    pub(super) fn render_attachments(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if self.attachments.is_empty() {
            return None;
        }
        let chips = self
            .attachments
            .iter()
            .enumerate()
            .map(|(index, attachment)| {
                h_flex()
                    .gap_1()
                    .pl_2()
                    .pr_0p5()
                    .py_0p5()
                    .rounded_md()
                    .bg(cx.theme().muted)
                    .text_xs()
                    .child(
                        Icon::new(match attachment {
                            Attachment::Image { .. } => IconName::Image,
                            Attachment::Text { .. } => IconName::FileText,
                            Attachment::Context { .. } => IconName::Bug,
                        })
                        .xsmall(),
                    )
                    .child(attachment.label())
                    .child(
                        Button::new(("remove-attachment", index))
                            .ghost()
                            .xsmall()
                            .icon(IconName::X)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if index < this.attachments.len() {
                                    this.attachments.remove(index);
                                }
                                cx.notify();
                            })),
                    )
            });
        Some(
            h_flex()
                .flex_wrap()
                .gap_1()
                .children(chips)
                .into_any_element(),
        )
    }

    pub(super) fn render_popup(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let popup = self.popup.as_ref()?;
        let label_font = if popup.kind == PopupKind::Mention {
            cx.theme().mono_font_family.clone()
        } else {
            cx.theme().font_family.clone()
        };
        let rows = popup.items.iter().enumerate().map(|(index, item)| {
            let selected = index == popup.selected;
            h_flex()
                .id(("popup-item", index))
                .w_full()
                .px_2()
                .py_1()
                .gap_2()
                .rounded_md()
                .cursor_pointer()
                .when(selected, |this| this.bg(cx.theme().accent))
                .hover(|this| this.bg(cx.theme().accent))
                .child(
                    div()
                        .flex_none()
                        .text_sm()
                        .font_family(label_font.clone())
                        .child(item.label.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(item.detail.clone()),
                )
                .on_click(
                    cx.listener(move |this, _, window, cx| this.pick_popup_item(index, window, cx)),
                )
        });
        let empty = popup.items.is_empty().then(|| {
            div()
                .p_2()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(if self.indexing {
                    "Indexing files…"
                } else {
                    "No matches"
                })
        });
        Some(
            v_flex()
                .id("composer-popup")
                .w_full()
                .max_h(px(280.))
                .overflow_y_scroll()
                .p_1()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().popover)
                .shadow_lg()
                .children(rows)
                .children(empty)
                .into_any_element(),
        )
    }

    /// For a thread that hasn't started: how the agents did in its project
    /// (green checks on the first try, and cost), to help pick one.
    pub(super) fn load_agent_hint(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        if !session.items.is_empty() {
            return;
        }
        let (project, name) = (session.project.id, session.project.name.clone());
        if let Some(rows) = crate::leaderboard::cached(project) {
            self.agent_hint = crate::leaderboard::hint(&rows, &name);
            return;
        }
        let Some(app) = session.app_entity() else {
            return;
        };
        let threads = crate::leaderboard::threads(app.read(cx), Some(project));
        let job = cx.background_executor().spawn(async move {
            crate::leaderboard::load(&elyra_core::paths::database_path(), threads)
        });
        cx.spawn(async move |this, cx| {
            let Ok(rows) = job.await else {
                return;
            };
            let hint = crate::leaderboard::hint(&rows, &name);
            crate::leaderboard::remember(project, rows);
            let _ = this.update(cx, |this, cx| {
                this.agent_hint = hint;
                cx.notify();
            });
        })
        .detach();
    }
}
