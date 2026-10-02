use crate::actions::{FindInThread, Interrupt};
use crate::composer::{self, Attachment, Popup, PopupKind};
use crate::thread_session::{SessionEvent, ThreadSession};
use crate::transcript;
use elyra_core::{
    ApprovalDecision, Environment, ItemContent, ItemId, PermissionMode, ProviderKind,
    QuestionAnswer, QuestionKind,
};
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{
    Input, InputEvent, InputState, RopeExt as _, Textarea, TextareaState,
};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

/// Answer state for an open question card.
pub struct QuestionForm {
    pub selected: Vec<HashSet<usize>>,
    /// "Other" / free-text answer per question.
    pub texts: Vec<Entity<InputState>>,
}

struct FindState {
    input: Entity<InputState>,
    matches: Vec<ItemId>,
    current: usize,
}

/// The conversation surface for one thread: transcript plus composer.
pub struct ThreadView {
    pub session: Entity<ThreadSession>,
    composer: Entity<TextareaState>,
    scroll: ScrollHandle,
    expanded: HashSet<ItemId>,
    forms: HashMap<String, QuestionForm>,
    attachments: Vec<Attachment>,
    popup: Option<Popup>,
    file_index: Option<Arc<Vec<String>>>,
    indexing: bool,
    /// Position while browsing earlier prompts with ↑/↓.
    history: Option<usize>,
    find: Option<FindState>,
    /// Follow new output while the user is at the bottom.
    follow: bool,
    is_git_repo: bool,
    warmed_up: bool,
    _subscriptions: Vec<Subscription>,
}

impl ThreadView {
    pub fn new(
        session: Entity<ThreadSession>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft = session.read(cx).draft.clone();
        let composer = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx)
                .auto_grow(2, 12)
                .submit_on_enter(true)
                .placeholder(
                    "Ask the agent to build, fix or explain…  (/ commands, @ files, Shift+Enter new line)",
                );
            if !draft.is_empty() {
                state.set_value(draft, window, cx);
            }
            state
        });
        let weak = cx.weak_entity();
        let subscriptions = vec![
            cx.subscribe_in(
                &composer,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { shift: false, .. } => this.submit(window, cx),
                    InputEvent::Change => this.composer_changed(cx),
                    _ => {}
                },
            ),
            cx.observe(&session, |this, _, cx| {
                if this.follow {
                    this.scroll.scroll_to_bottom();
                }
                cx.notify();
            }),
            cx.subscribe(&session, |_, _, event: &SessionEvent, cx| {
                if let SessionEvent::NeedsAttention = event {
                    cx.notify();
                }
            }),
            cx.intercept_keystrokes(move |event, window, cx| {
                let _ = weak.update(cx, |this, cx| this.intercept(event, window, cx));
            }),
        ];
        let is_git_repo = elyra_git::is_repo(&session.read(cx).project.path);
        let scroll = ScrollHandle::new();
        scroll.scroll_to_bottom();
        Self {
            session,
            composer,
            scroll,
            expanded: HashSet::new(),
            forms: HashMap::new(),
            attachments: Vec::new(),
            popup: None,
            file_index: None,
            indexing: false,
            history: None,
            find: None,
            follow: true,
            is_git_repo,
            warmed_up: false,
            _subscriptions: subscriptions,
        }
    }

    pub fn focus_composer(&self, window: &mut Window, cx: &mut App) {
        self.composer
            .update(cx, |composer, cx| composer.focus(window, cx));
    }

    fn composer_focused(&self, window: &Window, cx: &App) -> bool {
        self.composer.read(cx).focus_handle(cx).is_focused(window)
    }

    fn set_composer(
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

    fn intercept(&mut self, event: &KeystrokeEvent, window: &mut Window, cx: &mut Context<Self>) {
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

    fn browse_history(&mut self, back: bool, window: &mut Window, cx: &mut Context<Self>) -> bool {
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
    fn paste_special(&mut self, cx: &mut Context<Self>) -> bool {
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

    fn composer_changed(&mut self, cx: &mut Context<Self>) {
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
    }

    fn refresh_popup(&mut self, value: &str, cursor: usize, cx: &mut Context<Self>) {
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

    fn ensure_file_index(&mut self, cx: &mut Context<Self>) {
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

    fn move_popup(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(popup) = &mut self.popup {
            let len = popup.items.len() as isize;
            if len > 0 {
                popup.selected = (popup.selected as isize + delta).rem_euclid(len) as usize;
            }
        }
        cx.notify();
    }

    fn accept_popup(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn pick_popup_item(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(popup) = &mut self.popup {
            popup.selected = index;
        }
        self.accept_popup(window, cx);
    }

    // ---- attachments --------------------------------------------------------

    fn attach_paths(&mut self, paths: &[PathBuf], window: &mut Window, cx: &mut Context<Self>) {
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

    fn pick_files(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        let prompt = composer::build_prompt(&text, std::mem::take(&mut self.attachments));
        self.set_composer("", None, window, cx);
        self.history = None;
        self.follow = true;
        self.session
            .update(cx, |session, cx| session.submit(prompt, cx));
        self.scroll.scroll_to_bottom();
    }

    /// App-level slash commands. Returns true when handled.
    fn run_builtin(&mut self, text: &str, cx: &mut Context<Self>) -> bool {
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

    fn interrupt(&mut self, _: &Interrupt, _: &mut Window, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| session.interrupt(cx));
    }

    // ---- transcript actions ---------------------------------------------------

    pub fn toggle_expanded(&mut self, id: ItemId, cx: &mut Context<Self>) {
        if !self.expanded.remove(&id) {
            self.expanded.insert(id);
        }
        cx.notify();
    }

    pub fn respond(
        &mut self,
        request_id: &str,
        decision: ApprovalDecision,
        cx: &mut Context<Self>,
    ) {
        self.session
            .update(cx, |session, cx| session.respond(request_id, decision, cx));
    }

    pub fn approve_plan(
        &mut self,
        request_id: &str,
        mode: Option<PermissionMode>,
        cx: &mut Context<Self>,
    ) {
        self.session
            .update(cx, |session, cx| session.approve_plan(request_id, mode, cx));
    }

    pub fn answer(&mut self, request_id: &str, answer: QuestionAnswer, cx: &mut Context<Self>) {
        self.forms.remove(request_id);
        self.session
            .update(cx, |session, cx| session.answer(request_id, answer, cx));
    }

    pub fn toggle_option(
        &mut self,
        request_id: &str,
        question: usize,
        option: usize,
        multi: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(selected) = self
            .forms
            .get_mut(request_id)
            .and_then(|form| form.selected.get_mut(question))
        {
            if multi {
                if !selected.remove(&option) {
                    selected.insert(option);
                }
            } else {
                selected.clear();
                selected.insert(option);
            }
        }
        cx.notify();
    }

    pub fn submit_question(&mut self, request_id: &str, cx: &mut Context<Self>) {
        let request = self
            .session
            .read(cx)
            .items
            .iter()
            .find_map(|item| match &item.content {
                ItemContent::Question {
                    request,
                    answer: None,
                } if request.request_id == request_id => Some(request.clone()),
                _ => None,
            });
        let (Some(request), Some(form)) = (request, self.forms.get(request_id)) else {
            return;
        };
        let mut answers = Vec::new();
        for (qi, question) in request.questions.iter().enumerate() {
            let mut parts: Vec<String> = question
                .options
                .iter()
                .enumerate()
                .filter(|(oi, _)| form.selected.get(qi).is_some_and(|s| s.contains(oi)))
                .map(|(_, option)| option.label.clone())
                .collect();
            if let Some(text) = form.texts.get(qi) {
                let text = text.read(cx).value().trim().to_string();
                if !text.is_empty() {
                    parts.push(text);
                }
            }
            if parts.is_empty() {
                return; // every question needs an answer
            }
            answers.push(parts.join(", "));
        }
        self.answer(request_id, QuestionAnswer::Answers { answers }, cx);
    }

    pub fn confirm_restore(&mut self, sha: String, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.session.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (session, sha) = (session.clone(), sha.clone());
            alert
                .title("Restore files?")
                .description(
                    "Files in this thread's folder go back to how they were before this message: \
                     edits since then are undone and files created since are deleted. \
                     The conversation and commits are not changed.",
                )
                .confirm()
                .ok_text("Restore files")
                .on_ok(move |_, _, cx| {
                    session.update(cx, |s, cx| s.restore_checkpoint(sha.clone(), cx));
                    true
                })
        });
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

    /// Create answer state for questions that just arrived.
    fn ensure_forms(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open: Vec<_> = self
            .session
            .read(cx)
            .items
            .iter()
            .filter_map(|item| match &item.content {
                ItemContent::Question {
                    request,
                    answer: None,
                } => Some(request.clone()),
                _ => None,
            })
            .collect();
        for request in open {
            if self.forms.contains_key(&request.request_id) || request.kind == QuestionKind::Confirm
            {
                continue;
            }
            let texts = request
                .questions
                .iter()
                .map(|question| {
                    let placeholder = if request.kind == QuestionKind::Text {
                        "Type your answer"
                    } else {
                        "Other…"
                    };
                    let prefill = question.prefill.clone();
                    cx.new(|cx| {
                        let mut state = InputState::new(window, cx).placeholder(placeholder);
                        if !prefill.is_empty() {
                            state.set_value(prefill, window, cx);
                        }
                        state
                    })
                })
                .collect();
            self.forms.insert(
                request.request_id.clone(),
                QuestionForm {
                    selected: vec![HashSet::new(); request.questions.len()],
                    texts,
                },
            );
        }
    }

    // ---- find in thread -------------------------------------------------------

    fn open_find(&mut self, _: &FindInThread, window: &mut Window, cx: &mut Context<Self>) {
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

    fn close_find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find = None;
        self.focus_composer(window, cx);
        cx.notify();
    }

    fn update_find(&mut self, cx: &mut Context<Self>) {
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

    fn step_find(&mut self, delta: isize, cx: &mut Context<Self>) {
        if let Some(find) = &mut self.find {
            let len = find.matches.len() as isize;
            if len > 0 {
                find.current = (find.current as isize + delta).rem_euclid(len) as usize;
            }
        }
        self.scroll_to_match(cx);
    }

    fn scroll_to_match(&mut self, cx: &mut Context<Self>) {
        let target = self
            .find
            .as_ref()
            .and_then(|f| f.matches.get(f.current))
            .copied();
        if let Some(target) = target {
            self.follow = false;
            let rows =
                transcript::render(self.session.read(cx), &self.expanded, &self.forms, None, cx);
            if let Some(index) = rows.iter().position(|(id, _)| *id == Some(target)) {
                // Child 0 is the top spacer; the empty state is never shown with matches.
                self.scroll.scroll_to_item(index + 1);
            }
        }
        cx.notify();
    }

    fn update_follow(&mut self) {
        let offset = self.scroll.offset().y;
        let max = self.scroll.max_offset().y;
        // Offsets are negative when scrolled down.
        self.follow = (max + offset).abs() < px(48.);
    }

    // ---- rendering --------------------------------------------------------

    fn render_controls(&self, cx: &Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let caps = session.capabilities();
        let busy = session.running || session.preparing.is_some();
        let started = session.has_started();
        let provider = session.thread.provider;
        let current_model = session.thread.model.clone().unwrap_or_default();
        let model_label: SharedString = session
            .models
            .iter()
            .find(|m| m.id == current_model)
            .map(|m| m.label.clone())
            .unwrap_or_else(|| {
                if current_model.is_empty() {
                    "Default model".into()
                } else {
                    current_model.clone()
                }
            })
            .into();
        let models = session.models.clone();
        let prefs = crate::preferences::Preferences::global(cx);
        let enabled = prefs.enabled_providers();
        let custom_launch = prefs
            .launch(ProviderKind::CustomAcp, None)
            .executable
            .is_some();
        let presets = prefs.starred_models.clone();
        let accounts: Vec<String> = prefs
            .provider(provider)
            .accounts()
            .into_iter()
            .map(|(name, _)| name)
            .collect();
        let account = session.thread.account.clone();
        let current_preset = crate::preferences::ModelPreset {
            provider: provider.as_str().to_string(),
            model: current_model.clone(),
            effort: session.thread.effort.clone(),
        };
        let starred = prefs.is_starred(&current_preset);
        let weak_account = self.session.downgrade();
        let effort = session.thread.effort.clone().unwrap_or_default();
        let effort_label: SharedString = caps
            .efforts
            .iter()
            .find(|(id, _)| *id == effort)
            .map(|(_, label)| format!("Effort: {label}"))
            .unwrap_or_else(|| "Effort".into())
            .into();
        let efforts = caps.efforts;
        let mode = session.thread.permission_mode;
        let can_choose_env = !started
            && self.is_git_repo
            && matches!(session.thread.environment, Environment::Local);
        let env_label: SharedString = match &session.thread.environment {
            Environment::Local if session.use_worktree => "New worktree".into(),
            Environment::Local => "Local".into(),
            Environment::Worktree { branch, .. } => branch.clone().into(),
        };
        let env_icon =
            if session.use_worktree || !matches!(session.thread.environment, Environment::Local) {
                IconName::GitBranch
            } else {
                IconName::Folder
            };
        let has_text =
            !self.composer.read(cx).value().trim().is_empty() || !self.attachments.is_empty();
        let weak = self.session.downgrade();
        let (weak_model, weak_effort, weak_mode) = (weak.clone(), weak.clone(), weak.clone());

        // Options wrap on narrow panels; the meter and Send stay right.
        let options = h_flex()
            .flex_1()
            .min_w_0()
            .flex_wrap()
            .gap_1()
            .child(
                Button::new("attach")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Paperclip)
                    .tooltip("Attach files or images (or paste/drop them)")
                    .on_click(cx.listener(|this, _, window, cx| this.pick_files(window, cx))),
            )
            .child(
                Button::new("provider")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Bot)
                    .label(provider.label())
                    .dropdown_caret(!started)
                    .disabled(started)
                    .dropdown_menu(move |mut menu, _, _| {
                        for kind in enabled.iter().copied() {
                            let session = weak.clone();
                            let installed = elyra_provider::is_installed(kind)
                                || (kind == ProviderKind::CustomAcp && custom_launch);
                            let label = if installed {
                                kind.label().to_string()
                            } else {
                                format!("{} (not installed)", kind.label())
                            };
                            menu = menu.item(
                                PopupMenuItem::new(label)
                                    .checked(kind == provider)
                                    .disabled(!installed)
                                    .on_click(move |_, _, cx| {
                                        let _ =
                                            session.update(cx, |s, cx| s.set_provider(kind, cx));
                                    }),
                            );
                        }
                        menu
                    }),
            )
            .child(
                Button::new("model")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Sparkles)
                    .label(model_label)
                    .dropdown_caret(true)
                    .dropdown_menu(move |mut menu, _, _| {
                        menu = menu.scrollable(true).max_h(px(420.));
                        if !presets.is_empty() {
                            menu = menu.label("Starred");
                            for preset in &presets {
                                let kind = ProviderKind::parse(&preset.provider);
                                let label = format!(
                                    "★ {} · {}",
                                    kind.map(|k| k.label()).unwrap_or("?"),
                                    if preset.model.is_empty() {
                                        "Default"
                                    } else {
                                        &preset.model
                                    }
                                );
                                let other_provider = kind != Some(provider);
                                let session = weak_model.clone();
                                let preset = preset.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(label)
                                        .disabled(other_provider && started)
                                        .on_click(move |_, _, cx| {
                                            let _ = session
                                                .update(cx, |s, cx| s.apply_preset(&preset, cx));
                                        }),
                                );
                            }
                            menu = menu.separator();
                        }
                        for model in &models {
                            let session = weak_model.clone();
                            let id = model.id.clone();
                            menu = menu.item(
                                PopupMenuItem::new(model.label.clone())
                                    .checked(current_model == id)
                                    .on_click(move |_, _, cx| {
                                        let id = id.clone();
                                        let _ =
                                            session.update(cx, |s, cx| s.set_model(Some(id), cx));
                                    }),
                            );
                        }
                        let preset = current_preset.clone();
                        menu.separator().item(
                            PopupMenuItem::new(if starred {
                                "Unstar this model"
                            } else {
                                "Star this model"
                            })
                            .on_click(move |_, _, cx| {
                                let preset = preset.clone();
                                crate::preferences::update(cx, |p| {
                                    if p.is_starred(&preset) {
                                        p.starred_models.retain(|s| {
                                            !(s.provider == preset.provider
                                                && s.model == preset.model)
                                        });
                                    } else {
                                        p.starred_models.push(preset);
                                    }
                                });
                            }),
                        )
                    }),
            )
            .when(!accounts.is_empty(), |this| {
                let label: SharedString = account
                    .clone()
                    .unwrap_or_else(|| "Default account".into())
                    .into();
                this.child(
                    Button::new("account")
                        .ghost()
                        .xsmall()
                        .icon(IconName::CircleUser)
                        .label(label)
                        .dropdown_caret(!started)
                        .disabled(started)
                        .dropdown_menu(move |mut menu, _, _| {
                            let options =
                                std::iter::once(None).chain(accounts.iter().cloned().map(Some));
                            for option in options {
                                let session = weak_account.clone();
                                let label =
                                    option.clone().unwrap_or_else(|| "Default account".into());
                                menu = menu.item(
                                    PopupMenuItem::new(label)
                                        .checked(option == account)
                                        .on_click(move |_, _, cx| {
                                            let option = option.clone();
                                            let _ = session
                                                .update(cx, |s, cx| s.set_account(option, cx));
                                        }),
                                );
                            }
                            menu
                        }),
                )
            })
            .when(!efforts.is_empty(), |this| {
                this.child(
                    Button::new("effort")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Gauge)
                        .label(effort_label)
                        .dropdown_caret(true)
                        .dropdown_menu(move |mut menu, _, _| {
                            let options =
                                std::iter::once(("", "Default")).chain(efforts.iter().copied());
                            for (id, label) in options {
                                let session = weak_effort.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(label).checked(effort == id).on_click(
                                        move |_, _, cx| {
                                            let _ = session.update(cx, |s, cx| {
                                                s.set_effort(Some(id.to_string()), cx)
                                            });
                                        },
                                    ),
                                );
                            }
                            menu
                        }),
                )
            })
            .when(caps.permission_modes, |this| {
                this.child(
                    Button::new("permission")
                        .ghost()
                        .xsmall()
                        .icon(match mode {
                            PermissionMode::FullAccess => IconName::ShieldOff,
                            PermissionMode::Plan => IconName::NotebookPen,
                            _ => IconName::ShieldCheck,
                        })
                        .label(mode.label())
                        .dropdown_caret(true)
                        .dropdown_menu(move |mut menu, _, _| {
                            for option in PermissionMode::ALL {
                                let session = weak_mode.clone();
                                menu = menu.item(
                                    PopupMenuItem::new(option.label())
                                        .checked(option == mode)
                                        .on_click(move |_, _, cx| {
                                            let _ = session.update(cx, |s, cx| {
                                                s.set_permission_mode(option, cx)
                                            });
                                        }),
                                );
                            }
                            menu
                        }),
                )
            })
            .child(
                Button::new("environment")
                    .ghost()
                    .xsmall()
                    .icon(env_icon)
                    .label(env_label)
                    .disabled(!can_choose_env)
                    .tooltip(if can_choose_env {
                        "Toggle between the project checkout and an isolated Git worktree"
                    } else {
                        "Where this thread works"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session.update(cx, |session, cx| {
                            session.use_worktree = !session.use_worktree;
                            cx.notify();
                        });
                    })),
            );
        let actions = h_flex()
            .flex_none()
            .gap_1()
            .child(context_meter(session, cx))
            .when(busy, |this| {
                this.child(
                    Button::new("stop")
                        .small()
                        .danger()
                        .icon(IconName::Square)
                        .label("Stop")
                        .tooltip("Interrupt the agent (⌃C)")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.session.update(cx, |session, cx| session.interrupt(cx));
                        })),
                )
            })
            .when(!busy || has_text, |this| {
                this.child(
                    Button::new("send")
                        .small()
                        .primary()
                        .icon(if busy {
                            IconName::ListPlus
                        } else {
                            IconName::ArrowUp
                        })
                        .label(if busy { "Queue" } else { "Send" })
                        .disabled(!has_text)
                        .on_click(cx.listener(|this, _, window, cx| this.submit(window, cx))),
                )
            });
        h_flex()
            .w_full()
            .gap_1()
            .items_end()
            .child(options)
            .child(actions)
            .into_any_element()
    }

    fn render_interrupted(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let session = self.session.read(cx);
        if session.thread.status != elyra_core::ThreadStatus::Interrupted || session.running {
            return None;
        }
        Some(
            h_flex()
                .w_full()
                .px_3()
                .py_2()
                .gap_2()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().warning)
                .bg(cx.theme().warning.opacity(0.08))
                .text_sm()
                .child(
                    Icon::new(IconName::CirclePause)
                        .small()
                        .text_color(cx.theme().warning),
                )
                .child(
                    div()
                        .flex_1()
                        .child("The last turn was interrupted when the app quit."),
                )
                .child(
                    Button::new("resume-interrupted")
                        .small()
                        .primary()
                        .label("Resume")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.session.update(cx, |s, cx| s.resume_interrupted(cx));
                        })),
                )
                .child(
                    Button::new("dismiss-interrupted")
                        .small()
                        .ghost()
                        .label("Dismiss")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.session.update(cx, |s, cx| s.dismiss_interrupted(cx));
                        })),
                )
                .into_any_element(),
        )
    }

    fn render_queue(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let session = self.session.read(cx);
        if session.queued.is_empty() && session.provider_queue.is_empty() {
            return None;
        }
        let can_steer = session.capabilities().steer && session.running;
        let rows = session.queued.iter().enumerate().map(|(index, prompt)| {
            let preview: String = prompt
                .text
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(120)
                .collect();
            h_flex()
                .id(("queued", index))
                .gap_2()
                .text_sm()
                .child(
                    Icon::new(IconName::Clock)
                        .xsmall()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(preview),
                )
                .when(can_steer, |this| {
                    this.child(
                        Button::new(("steer", index))
                            .ghost()
                            .xsmall()
                            .label("Send now")
                            .tooltip("Deliver into the running turn")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.session.update(cx, |s, cx| s.steer_queued(index, cx));
                            })),
                    )
                })
                .child(
                    Button::new(("edit-queued", index))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Pencil)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let prompt =
                                this.session.update(cx, |s, cx| s.remove_queued(index, cx));
                            if let Some(prompt) = prompt {
                                this.edit_message(prompt.text, window, cx);
                            }
                        })),
                )
                .child(
                    Button::new(("drop-queued", index))
                        .ghost()
                        .xsmall()
                        .icon(IconName::X)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.session.update(cx, |s, cx| s.remove_queued(index, cx));
                        })),
                )
        });
        let pending = session.provider_queue.iter().map(|text| {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(format!(
                    "↳ steering: {}",
                    text.chars().take(120).collect::<String>()
                ))
        });
        Some(
            v_flex()
                .w_full()
                .px_2()
                .gap_1()
                .children(rows)
                .children(pending)
                .into_any_element(),
        )
    }

    fn render_attachments(&self, cx: &Context<Self>) -> Option<AnyElement> {
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
                        Icon::new(if attachment.is_image() {
                            IconName::Image
                        } else {
                            IconName::FileText
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

    fn render_popup(&self, cx: &Context<Self>) -> Option<AnyElement> {
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

    fn render_find(&self, cx: &Context<Self>) -> Option<AnyElement> {
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

fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.)
    } else if tokens >= 1000 {
        format!("{}k", tokens / 1000)
    } else {
        tokens.to_string()
    }
}

fn context_meter(session: &ThreadSession, cx: &App) -> AnyElement {
    let (Some(used), Some(window)) = (session.usage.context_tokens, session.usage.context_window)
    else {
        return div().into_any_element();
    };
    let ratio = (used as f32 / window.max(1) as f32).clamp(0., 1.);
    let color = if ratio > 0.85 {
        cx.theme().danger
    } else if ratio > 0.6 {
        cx.theme().warning
    } else {
        cx.theme().muted_foreground
    };
    let cost = session
        .usage
        .cost_usd
        .map(|c| format!(" · ${c:.2}"))
        .unwrap_or_default();
    let tooltip = format!(
        "Context: {} of {} tokens{cost}",
        format_tokens(used),
        format_tokens(window)
    );
    h_flex()
        .id("context-meter")
        .gap_1()
        .px_1()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(
            div()
                .w(px(36.))
                .h(px(4.))
                .rounded_full()
                .bg(cx.theme().muted)
                .child(
                    div()
                        .h_full()
                        .rounded_full()
                        .w(relative(ratio.max(0.03)))
                        .bg(color),
                ),
        )
        .child(format!("{}%", (ratio * 100.).round() as u32))
        .tooltip(move |window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(tooltip.clone()).build(window, cx)
        })
        .into_any_element()
}

impl Render for ThreadView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_forms(window, cx);
        let session = self.session.read(cx);
        let empty = session.items.is_empty() && !session.running && session.preparing.is_none();
        let project_name = session.project.name.clone();
        let cwd = session.working_dir().display().to_string();
        let highlight = self
            .find
            .as_ref()
            .and_then(|f| f.matches.get(f.current))
            .copied();
        let rows = transcript::render(
            self.session.read(cx),
            &self.expanded,
            &self.forms,
            highlight,
            cx,
        );
        let controls = self.render_controls(cx);
        let queue = self.render_queue(cx);
        let interrupted = self.render_interrupted(cx);
        let attachments = self.render_attachments(cx);
        let popup = self.render_popup(cx);
        let find = self.render_find(cx);

        let prefs = crate::preferences::Preferences::global(cx);
        let width = prefs.chat_width;
        let compact = prefs.compact();
        let row = |child: AnyElement| {
            div()
                .w_full()
                .when(width > 0., |this| this.max_w(px(width)))
                .mx_auto()
                .when(compact, |this| this.px_4().py_0p5())
                .when(!compact, |this| this.px_6().py_1p5())
                .child(child)
        };
        let mut children: Vec<AnyElement> = vec![div().h(px(12.)).into_any_element()];
        if empty {
            children.push(
                row(v_flex()
                    .pt(px(120.))
                    .items_center()
                    .gap_2()
                    .child(
                        Icon::new(IconName::Bot)
                            .large()
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(format!("What should we do in {project_name}?")),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(cwd),
                    )
                    .into_any_element())
                .into_any_element(),
            );
        }
        children.extend(
            rows.into_iter()
                .map(|(_, element)| row(element).into_any_element()),
        );
        children.push(div().h(px(16.)).into_any_element());

        v_flex()
            .key_context("Composer")
            .on_action(cx.listener(Self::interrupt))
            .on_action(cx.listener(Self::open_find))
            .size_full()
            .children(find)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("transcript")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .on_scroll_wheel(cx.listener(|this, _, _, _| this.update_follow()))
                            .children(children),
                    )
                    .vertical_scrollbar(&self.scroll),
            )
            .child(
                div().w_full().px_6().pb_4().child(
                    v_flex()
                        .id("composer-box")
                        .w_full()
                        .when(width > 0., |this| this.max_w(px(width)))
                        .mx_auto()
                        .gap_1()
                        .children(interrupted)
                        .children(popup)
                        .children(queue)
                        .child(
                            v_flex()
                                .id("composer-frame")
                                .w_full()
                                .p_2()
                                .gap_2()
                                .rounded_xl()
                                .border_1()
                                .border_color(cx.theme().border)
                                .bg(cx.theme().background)
                                .shadow_sm()
                                .drag_over::<ExternalPaths>(|style, _, _, cx| {
                                    style.border_color(cx.theme().ring)
                                })
                                .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                                    this.attach_paths(paths.paths(), window, cx)
                                }))
                                .children(attachments)
                                .child(Textarea::new(&self.composer).appearance(false))
                                .child(controls),
                        ),
                ),
            )
    }
}
