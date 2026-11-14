//! While you were away: what the threads did since you last looked.

use super::*;

impl Workspace {
    /// Back at the window: after a while away, say what happened meanwhile.
    pub(super) fn came_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(since) = self.away_since.take() else {
            return;
        };
        let now = chrono::Utc::now();
        if now - since >= crate::away::AWAY {
            self.last_away = Some((since, now));
            self.show_away(since, now, window, cx);
        }
    }

    /// What the threads did between `from` and `to`; false when nothing.
    pub(crate) fn show_away(
        &mut self,
        from: chrono::DateTime<chrono::Utc>,
        to: chrono::DateTime<chrono::Utc>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let mut entries: Vec<crate::away::Entry> = Vec::new();
        {
            let state = self.app.read(cx);
            for thread in &state.threads {
                let recent = thread
                    .last_activity_at
                    .unwrap_or(thread.updated_at)
                    .max(thread.updated_at)
                    >= from;
                let session = state.existing_session(thread.id);
                let running = session.as_ref().is_some_and(|s| s.read(cx).running);
                if !recent && !running {
                    continue;
                }
                let items = match &session {
                    Some(session) => session.read(cx).items.clone(),
                    None => state.store.transcript(thread.id).unwrap_or_default(),
                };
                let project = state
                    .project(thread.project_id)
                    .map(|p| p.name.clone())
                    .unwrap_or_default();
                if let Some(entry) = crate::away::summarize(thread, &project, &items, from, running)
                {
                    entries.push(entry);
                }
            }
        }
        if entries.is_empty() {
            return false;
        }
        entries.sort_by_key(|e| e.state);
        let total: f64 = entries.iter().map(|e| e.cost).sum();
        let title = format!("While you were away ({})", crate::away::duration(to - from));
        let workspace = cx.weak_entity();
        window.open_dialog(cx, move |dialog, _, cx| {
            let rows = entries.iter().map(|entry| {
                let (icon, color) = match entry.state {
                    crate::away::State::NeedsYou => (IconName::CircleAlert, cx.theme().warning),
                    crate::away::State::Failed => (IconName::CircleX, cx.theme().danger),
                    crate::away::State::Finished => (IconName::CircleCheck, cx.theme().success),
                    crate::away::State::Working => (IconName::LoaderCircle, cx.theme().info),
                };
                let id = entry.thread;
                let workspace = workspace.clone();
                h_flex()
                    .id(SharedString::from(format!("away-{id}")))
                    .w_full()
                    .items_start()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|this| this.bg(cx.theme().muted))
                    .child(Icon::new(icon).small().text_color(color))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .text_sm()
                                    .child(
                                        div()
                                            .min_w_0()
                                            .overflow_hidden()
                                            .text_ellipsis()
                                            .whitespace_nowrap()
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(entry.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .flex_none()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(entry.project.clone()),
                                    ),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .whitespace_normal()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(entry.line.clone()),
                            ),
                    )
                    .when(entry.cost > 0.0, |this| {
                        this.child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(crate::thread_session::format_usd(entry.cost)),
                        )
                    })
                    .on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        let _ = workspace.update(cx, |this, cx| this.activate(id, window, cx));
                    })
            });
            dialog
                .title(title.clone())
                .w(px(560.))
                .child(v_flex().gap_1().children(rows))
                .footer(
                    h_flex()
                        .w_full()
                        .justify_between()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(if total > 0.0 {
                                    format!(
                                        "Spent meanwhile: {}",
                                        crate::thread_session::format_usd(total)
                                    )
                                } else {
                                    String::new()
                                }),
                        )
                        .child(
                            Button::new("away-close")
                                .small()
                                .label("Close")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        ),
                )
        });
        true
    }
}
