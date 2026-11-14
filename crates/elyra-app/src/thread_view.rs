//! A thread's view: the transcript, the composer under it and the controls
//! around them. This module has the state and setting it up; beside it are
//! `compose` (the composer, its popups and sending), `cards` (what the
//! transcript's cards do), `rows` (the transcript as a list), `find` (⌘F),
//! `app_errors` (errors from the app being built) and `render`.

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

mod app_errors;
mod cards;
mod compose;
mod find;
mod render;
mod rows;

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

/// Whether Félagi is connected with a token that may write.
fn felagi_can_write(cx: &App) -> bool {
    crate::preferences::app_state(cx)
        .and_then(|app| crate::felagi::connection(&app.read(cx).store))
        .is_some_and(|c| c.can_write)
}

/// How often to ask Grove for new server errors in the project's app.
const SERVER_ERROR_CHECK: std::time::Duration = std::time::Duration::from_secs(3);

/// At most this many server errors are explained in one message.
const MAX_SERVER_ERRORS: usize = 3;

/// The conversation surface for one thread: transcript plus composer.
pub struct ThreadView {
    pub session: Entity<ThreadSession>,
    composer: Entity<TextareaState>,
    /// The conversation, drawn row by row: only the rows in view are built
    /// each frame. Follows the end while it's at the bottom.
    list: ListState,
    /// Which entries have rows, and where results and subagent steps are.
    rows: transcript::RowIndex,
    /// The reply being streamed, parsed as it grows: (its state, how much
    /// of the text it has, the number of entries when it began).
    reply: Option<(
        Entity<gpui_kit::component::text::TextViewState>,
        usize,
        usize,
    )>,
    /// For a new thread: how the agents have done in its project.
    agent_hint: Option<String>,
    /// For a new thread's first message: the agent that did best in the
    /// areas it names, with the message it was worked out for.
    route: Option<(String, crate::routing::Pick)>,
    _route_task: Option<Task<()>>,
    expanded: HashSet<ItemId>,
    forms: HashMap<String, QuestionForm>,
    attachments: Vec<Attachment>,
    popup: Option<Popup>,
    file_index: Option<Arc<Vec<String>>>,
    indexing: bool,
    /// Position while browsing earlier prompts with ↑/↓.
    history: Option<usize>,
    find: Option<FindState>,
    is_git_repo: bool,
    warmed_up: bool,
    /// The branch the thread's folder is on (asked in the background).
    branch: Option<String>,
    /// The Félagi issue the thread works on, with its banner.
    felagi: Option<Entity<crate::felagi_report::FelagiLink>>,
    /// The thread's browser, for the chip that offers its new errors.
    browser: Option<Entity<crate::browser_view::BrowserView>>,
    /// Server errors (5xx) Grove recorded for the project's app since the
    /// thread was opened, not yet added to a message or dismissed.
    server_errors: Vec<crate::grove::Request>,
    /// The newest Grove request id already looked at.
    server_seen: Option<u64>,
    /// Errors the Laravel app logged since, traced to the instruction that
    /// wrote the failing line; not yet added to a message or dismissed.
    app_errors: Vec<crate::app_errors::Traced>,
    /// How far the app's log was read.
    log_seen: Option<crate::app_errors::Seen>,
    _grove_watch: Task<()>,
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
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.subscribe(&session, |this, _, event: &SessionEvent, cx| match event {
                SessionEvent::NeedsAttention | SessionEvent::Streamed => cx.notify(),
                // The agent may have switched branches.
                SessionEvent::TurnCompleted => this.refresh_branch(cx),
                _ => {}
            }),
            cx.intercept_keystrokes(move |event, window, cx| {
                let _ = weak.update(cx, |this, cx| this.intercept(event, window, cx));
            }),
        ];
        let grove_watch = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(SERVER_ERROR_CHECK).await;
                // The folder the thread works in: a worktree Grove runs has
                // its own site.
                let Ok((dir, recent)) = this.update(cx, |this, cx| {
                    let session = this.session.read(cx);
                    // Server errors matter while the agent works and shortly
                    // after; an idle tab doesn't ask Grove every few seconds.
                    let recent = session.running
                        || session.thread.last_activity_at.is_some_and(|at| {
                            chrono::Utc::now() - at < chrono::Duration::minutes(30)
                        });
                    (session.working_dir(), recent)
                }) else {
                    break;
                };
                if !recent {
                    continue;
                }
                let Ok(seen) = this.update(cx, |this, _| this.log_seen.clone()) else {
                    break;
                };
                let (requests, (seen, logged)) = cx
                    .background_executor()
                    .spawn(async move {
                        // The app's own log, and the line each error failed on.
                        let (seen, errors) = crate::app_errors::new_errors(&dir, seen);
                        let logged: Vec<_> = errors
                            .into_iter()
                            .rev()
                            .take(MAX_SERVER_ERRORS)
                            .map(|error| {
                                let culprit = elyra_git::repo_root(&dir).ok().and_then(|root| {
                                    crate::app_errors::culprit(&root, &error).map(|c| (root, c))
                                });
                                (error, culprit)
                            })
                            .collect();
                        let requests = crate::grove::app_for(&dir)
                            .and_then(|site| crate::grove::requests(&site.name, 30).ok());
                        (requests, (seen, logged))
                    })
                    .await;
                let alive = this.update(cx, |this, cx| {
                    if let Some(requests) = requests {
                        this.saw_requests(requests, cx);
                    }
                    this.log_seen = seen;
                    this.saw_app_errors(logged, cx);
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        let felagi = session.read(cx).thread.felagi_issue.clone().map(|id| {
            let session = session.clone();
            cx.new(|cx| crate::felagi_report::FelagiLink::new(session, id, cx))
        });
        // Rows above and below the visible ones are drawn ahead, for smooth
        // scrolling.
        let list = ListState::new(FIRST_ROW + TRAILING_ROWS, ListAlignment::Top, px(1000.));
        list.set_follow_mode(FollowMode::Tail);
        let mut this = Self {
            session,
            composer,
            list,
            rows: transcript::RowIndex::default(),
            reply: None,
            agent_hint: None,
            route: None,
            _route_task: None,
            expanded: HashSet::new(),
            forms: HashMap::new(),
            attachments: Vec::new(),
            popup: None,
            file_index: None,
            indexing: false,
            history: None,
            find: None,
            // Found out in the background, with the branch.
            is_git_repo: false,
            felagi,
            branch: None,
            browser: None,
            server_errors: Vec::new(),
            server_seen: None,
            app_errors: Vec::new(),
            log_seen: None,
            _grove_watch: grove_watch,
            warmed_up: false,
            _subscriptions: subscriptions,
        };
        this.refresh_branch(cx);
        this.load_agent_hint(cx);
        this
    }

    fn refresh_branch(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        if let Environment::Worktree { branch, .. } = &session.thread.environment {
            self.branch = Some(branch.clone());
            return;
        }
        let dir = session.working_dir();
        let project = session.project.path.clone();
        let job = cx.background_executor().spawn(async move {
            (
                elyra_git::current_branch(&dir),
                elyra_git::is_repo(&project),
            )
        });
        cx.spawn(async move |this, cx| {
            let (branch, is_repo) = job.await;
            let _ = this.update(cx, |this, cx| {
                if this.branch != branch || this.is_git_repo != is_repo {
                    this.branch = branch;
                    this.is_git_repo = is_repo;
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

pub(crate) fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.)
    } else if tokens >= 1000 {
        format!("{}k", tokens / 1000)
    } else {
        tokens.to_string()
    }
}

/// The list's first entry row (row 0 is the top spacer or empty state).
const FIRST_ROW: usize = 1;
/// Rows after the entries: the tail and the bottom spacer.
const TRAILING_ROWS: usize = 2;
