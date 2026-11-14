//! Find in the thread (⌘F).

use super::*;

impl ThreadView {
    pub(super) fn open_find(
        &mut self,
        _: &FindInThread,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(find) = &self.find {
            find.input.update(cx, |input, cx| input.focus(window, cx));
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Find in thread"));
        self._subscriptions.push(cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => this.update_find(cx),
                InputEvent::PressEnter { shift, .. } => {
                    this.step_find(if *shift { -1 } else { 1 }, cx)
                }
                _ => {}
            },
        ));
        input.update(cx, |input, cx| input.focus(window, cx));
        self.find = Some(FindState {
            input,
            matches: Vec::new(),
            current: 0,
        });
        cx.notify();
    }

    pub(super) fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find = None;
        self.focus_composer(window, cx);
        cx.notify();
    }

    pub(super) fn update_find(&mut self, cx: &mut Context<Self>) {
        let Some(find) = &mut self.find else {
            return;
        };
        let query = find.input.read(cx).value().to_lowercase();
        find.matches = if query.trim().is_empty() {
            Vec::new()
        } else {
            self.session
                .read(cx)
                .items
                .iter()
                .filter(|item| {
                    transcript::searchable_text(item)
                        .is_some_and(|text| text.to_lowercase().contains(&query))
                })
                .map(|item| item.id)
                .collect()
        };
        find.current = find.matches.len().saturating_sub(1);
        self.scroll_to_match(cx);
    }

    pub(super) fn step_find(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(find) = &mut self.find {
            let len = find.matches.len() as isize;
            if len > 0 {
                find.current = (find.current as isize + delta).rem_euclid(len) as usize;
            }
        }
        self.scroll_to_match(cx);
    }

    /// Scroll the conversation to an entry (a tool result or a subagent's
    /// step: to the call it belongs to).
    pub fn reveal(&mut self, item: elyra_core::ItemId, cx: &mut Context<Self>) {
        self.sync_rows(cx);
        let row = self.rows.row_of(&self.session.read(cx).items, item);
        if let Some(row) = row {
            // Stay there rather than snap back to the end; scrolling back
            // down follows again.
            self.list.pause_following_tail();
            self.list.scroll_to_reveal_item(FIRST_ROW + row);
        }
        cx.notify();
    }

    pub(super) fn scroll_to_match(&mut self, cx: &mut Context<Self>) {
        let target = self
            .find
            .as_ref()
            .and_then(|f| f.matches.get(f.current))
            .copied();
        if let Some(target) = target {
            self.reveal(target, cx);
        }
        cx.notify();
    }

    pub(super) fn render_find(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let find = self.find.as_ref()?;
        let count = if find.matches.is_empty() {
            "No results".to_string()
        } else {
            format!("{} of {}", find.current + 1, find.matches.len())
        };
        Some(
            h_flex()
                .w_full()
                .px_3()
                .py_1()
                .gap_2()
                .border_b_1()
                .border_color(cx.theme().border)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        this.close_find(window, cx);
                        cx.stop_propagation();
                    }
                }))
                .child(
                    Icon::new(IconName::Search)
                        .small()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    div()
                        .flex_1()
                        .child(Input::new(&find.input).small().appearance(false)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(count),
                )
                .child(
                    Button::new("find-prev")
                        .ghost()
                        .xsmall()
                        .icon(IconName::ChevronUp)
                        .on_click(cx.listener(|this, _, _, cx| this.step_find(-1, cx))),
                )
                .child(
                    Button::new("find-next")
                        .ghost()
                        .xsmall()
                        .icon(IconName::ChevronDown)
                        .on_click(cx.listener(|this, _, _, cx| this.step_find(1, cx))),
                )
                .child(
                    Button::new("find-close")
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .on_click(cx.listener(|this, _, window, cx| this.close_find(window, cx))),
                )
                .into_any_element(),
        )
    }
}
