//! The transcript as a list: its rows kept in step with the session, and
//! drawing one.

use super::*;

impl ThreadView {
    /// Hand the newly streamed part of the reply to its text state, which
    /// parses only that, instead of the whole reply on every chunk.
    pub(super) fn sync_reply(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let (text, entries) = (&session.streaming_text, session.items.len());
        if text.is_empty() {
            self.reply = None;
            return;
        }
        match &mut self.reply {
            // The same reply, grown.
            Some((state, len, began))
                if *began == entries && text.len() >= *len && text.is_char_boundary(*len) =>
            {
                if text.len() > *len {
                    let added = text[*len..].to_string();
                    *len = text.len();
                    state.update(cx, |state, cx| state.push_str(&added, cx));
                }
            }
            // A new one.
            _ => {
                let (text, len) = (text.clone(), text.len());
                let state =
                    cx.new(|cx| gpui_kit::component::text::TextViewState::markdown(&text, cx));
                self.reply = Some((state, len, entries));
            }
        }
    }

    /// Bring the list in step with the transcript: new entries are added
    /// after the existing rows, anything else lays the list out again.
    pub(super) fn sync_rows(&mut self, cx: &App) {
        let session = self.session.read(cx);
        match self.rows.update(&session.items) {
            transcript::RowChange::Unchanged => {}
            transcript::RowChange::Appended { from, count } => {
                let at = FIRST_ROW + from;
                self.list.splice(at..at, count);
            }
            transcript::RowChange::Rebuilt => {
                self.list
                    .reset(self.rows.rows.len() + FIRST_ROW + TRAILING_ROWS);
            }
        }
    }

    /// One row of the list: the top (a spacer, or the empty state), an
    /// entry, the tail (what is streaming), or the bottom spacer.
    pub(super) fn render_list_row(
        &mut self,
        ix: usize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let session = self.session.read(cx);
        let count = self.rows.rows.len();
        if ix == 0 {
            let empty = session.items.is_empty() && !session.running && session.preparing.is_none();
            return if empty {
                let project_name = session.project.name.clone();
                let cwd = session.working_dir().display().to_string();
                self.frame(self.empty_state(project_name, cwd, cx), cx)
            } else {
                div().h(px(12.)).into_any_element()
            };
        }
        if ix == FIRST_ROW + count + 1 {
            return div().h(px(16.)).into_any_element();
        }
        if ix == FIRST_ROW + count {
            let tail =
                transcript::tail(session, self.reply.as_ref().map(|(state, _, _)| state), cx);
            return if tail.is_empty() {
                div().into_any_element()
            } else {
                self.frame(v_flex().gap_2().children(tail).into_any_element(), cx)
            };
        }
        let highlight = self
            .find
            .as_ref()
            .and_then(|f| f.matches.get(f.current))
            .copied();
        let Some(&position) = self.rows.rows.get(ix - FIRST_ROW) else {
            return div().into_any_element();
        };
        match transcript::row(
            session,
            &self.rows,
            position,
            &self.expanded,
            &self.forms,
            highlight,
            cx,
        ) {
            Some(element) => self.frame(element, cx),
            None => div().into_any_element(),
        }
    }

    /// A row's frame: the chat width and spacing from the preferences.
    pub(super) fn frame(&self, child: AnyElement, cx: &App) -> AnyElement {
        let prefs = crate::preferences::Preferences::global(cx);
        let (width, compact) = (prefs.chat_width, prefs.compact());
        div()
            .w_full()
            .when(width > 0., |this| this.max_w(px(width)))
            .mx_auto()
            .when(compact, |this| this.px_4().py_0p5())
            .when(!compact, |this| this.px_6().py_1p5())
            .child(child)
            .into_any_element()
    }

    // ---- rendering --------------------------------------------------------
}
