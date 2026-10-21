//! Red vs. Blue: a duel between two agents over one requirement, so the
//! tests don't share the code's blind spots. Red writes the tests, in a new
//! worktree: only test files (anything else is put back), and they must fail
//! against the current code (Elyra runs the checks to see that). Then Blue
//! gets them, locked, in the same worktree, and writes the code until the
//! checks are green; a locked test Blue changes is put back. When Blue is
//! sure a test is wrong, it challenges it (the gateway's `challenge_test`):
//! Red answers, and may change it.
//!
//! A duel is kept in the settings table (`duels`), so it carries on after a
//! restart.

use crate::app_state::AppState;
use crate::thread_session::{SessionEvent, ThreadSession};
use elyra_core::{ItemContent, ProjectId, ProviderKind, Store, ThreadId};
use elyra_provider::Prompt;
use gpui_kit::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use uuid::Uuid;

const KEY: &str = "duels";
/// How often Red may try for tests that fail, Blue's locked tests be put
/// back, and Blue challenge a test.
const MAX_TRIES: u32 = 3;
const MAX_RESTORES: u32 = 3;
pub const MAX_CHALLENGES: u32 = 3;

/// The start of the messages Elyra sends (Blue's, for telling them apart).
pub const RED: &str = "You are Red in a duel";
pub const BLUE: &str = "You are Blue in a duel";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Red writes the tests.
    Specifying,
    /// Blue writes the code.
    Implementing,
    /// The checks are green with Red's tests.
    Done,
    /// It stopped (too many tries); the threads say why.
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Duel {
    pub id: Uuid,
    pub project: ProjectId,
    pub requirement: String,
    pub red: ThreadId,
    pub blue_provider: ProviderKind,
    #[serde(default)]
    pub blue: Option<ThreadId>,
    pub phase: Phase,
    /// The tests Red wrote, locked for Blue.
    #[serde(default)]
    pub tests: Vec<String>,
    /// The snapshot holding them as Red left them.
    #[serde(default)]
    pub locked: Option<String>,
    #[serde(default)]
    pub tries: u32,
    #[serde(default)]
    pub restores: u32,
    #[serde(default)]
    pub challenges: u32,
}

// ---- prompts ----------------------------------------------------------------------

pub fn red_prompt(requirement: &str) -> String {
    format!(
        "{RED}: you write the tests, and another agent (Blue) then writes the code that makes them pass.\n\nThe requirement:\n{}\n\nWrite tests in the project's own test suite that pin this requirement down, with the edge cases a quick implementation would miss. Only add or change test files: Elyra puts anything else back. Don't implement anything. The tests must fail against the current code; Elyra runs the checks to see that they do, then hands them to Blue, locked.",
        requirement.trim()
    )
}

pub fn blue_prompt(requirement: &str, tests: &[String]) -> String {
    format!(
        "{BLUE}: another agent (Red) wrote tests for a requirement, and you write the code that makes them pass.\n\nThe requirement:\n{}\n\nRed's tests, locked (Elyra puts them back if they change):\n{}\n\nMake the project's checks pass without changing those tests. If you are sure a test is wrong (it contradicts the requirement), call the gateway tool challenge_test with the file, the test and why: Red answers, and may change it.",
        requirement.trim(),
        tests
            .iter()
            .map(|t| format!("- {t}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

fn challenge_prompt(file: &str, test: &str, reason: &str) -> String {
    format!(
        "Blue challenges your test “{test}” in {file}:\n\n{}\n\nIf Blue is right, fix the test (only test files) and start your reply with AGREED. If the test is right, start with DISAGREED and say why, pointing at the requirement.",
        reason.trim()
    )
}

// ---- keeping duels ------------------------------------------------------------------

pub fn all(store: &Store) -> Vec<Duel> {
    store
        .setting(KEY)
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default()
}

fn save(store: &Store, duel: &Duel) {
    let mut duels = all(store);
    match duels.iter_mut().find(|d| d.id == duel.id) {
        Some(existing) => *existing = duel.clone(),
        None => duels.push(duel.clone()),
    }
    let saved = serde_json::to_string(&duels)
        .map_err(anyhow::Error::from)
        .and_then(|json| store.set_setting(KEY, &json));
    if let Err(err) = saved {
        log::warn!("saving a duel: {err:#}");
    }
}

/// The duel a thread is Red or Blue in.
pub fn of_thread(store: &Store, thread: ThreadId) -> Option<Duel> {
    all(store)
        .into_iter()
        .find(|d| d.red == thread || d.blue == Some(thread))
}

// ---- watching -----------------------------------------------------------------------

#[derive(Default)]
struct Watching {
    subscriptions: HashMap<ThreadId, Subscription>,
    /// Duels whose turn is being looked at.
    busy: HashSet<Uuid>,
}

impl Global for Watching {}

/// Carry on with the duels in progress.
pub fn init(app: Entity<AppState>, cx: &mut App) {
    cx.set_global(Watching::default());
    for duel in all(&app.read(cx).store) {
        if matches!(duel.phase, Phase::Specifying | Phase::Implementing)
            && let Some(red) = app.update(cx, |app, cx| app.session(duel.red, cx))
        {
            red.update(cx, |red, _| red.skip_checks = true);
        }
        match duel.phase {
            Phase::Specifying => watch(&app, duel.red, cx),
            Phase::Implementing => {
                watch(&app, duel.red, cx);
                if let Some(blue) = duel.blue {
                    watch(&app, blue, cx);
                }
            }
            Phase::Done | Phase::Stopped => {}
        }
    }
}

fn watch(app: &Entity<AppState>, thread: ThreadId, cx: &mut App) {
    if cx.global::<Watching>().subscriptions.contains_key(&thread) {
        return;
    }
    let Some(session) = app.update(cx, |app, cx| app.session(thread, cx)) else {
        return;
    };
    let app = app.clone();
    let subscription = cx.subscribe(&session, move |session, event: &SessionEvent, cx| {
        if !matches!(event, SessionEvent::TurnCompleted) {
            return;
        }
        let settled = {
            let session = session.read(cx);
            !session.running && session.queued.is_empty()
        };
        if settled {
            turn_ended(&app, thread, cx);
        }
    });
    cx.update_global::<Watching, _>(|w, _| {
        w.subscriptions.insert(thread, subscription);
    });
}

/// Start a duel: Red's thread, in a new worktree, gets the requirement.
pub fn start(
    app: &Entity<AppState>,
    project: ProjectId,
    requirement: String,
    red: ProviderKind,
    blue: ProviderKind,
    cx: &mut App,
) -> Result<ThreadId, String> {
    let requirement = requirement.trim().to_string();
    if requirement.is_empty() {
        return Err("Write the requirement first.".into());
    }
    let has_check = app
        .read(cx)
        .project(project)
        .and_then(|p| p.check_command.clone())
        .is_some_and(|c| !c.trim().is_empty());
    if !has_check {
        return Err("A duel needs the project's check command (Context tab): it is how Red's tests are run.".into());
    }
    let thread = app
        .update(cx, |app, cx| app.create_thread(project, cx))
        .map_err(|err| format!("{err:#}"))?;
    let session = app
        .update(cx, |app, cx| app.session(thread.id, cx))
        .ok_or("could not open the thread")?;
    let short: String = requirement
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(40)
        .collect();
    session.update(cx, |session, cx| {
        session.set_provider(red, cx);
        // Red's tests are meant to fail: no automatic fixes for them.
        session.skip_checks = true;
        session.use_worktree = true;
        session.rename(format!("Red · tests · {short}"), cx);
        session.submit(Prompt::text(red_prompt(&requirement)), cx);
    });
    let duel = Duel {
        id: Uuid::new_v4(),
        project,
        requirement,
        red: thread.id,
        blue_provider: blue,
        blue: None,
        phase: Phase::Specifying,
        tests: Vec::new(),
        locked: None,
        tries: 0,
        restores: 0,
        challenges: 0,
    };
    save(&app.read(cx).store, &duel);
    watch(app, thread.id, cx);
    Ok(thread.id)
}

fn turn_ended(app: &Entity<AppState>, thread: ThreadId, cx: &mut App) {
    let Some(duel) = of_thread(&app.read(cx).store, thread) else {
        return;
    };
    if cx.global::<Watching>().busy.contains(&duel.id) {
        return;
    }
    match duel.phase {
        Phase::Specifying if duel.red == thread => red_done(app, duel, cx),
        Phase::Implementing if duel.blue == Some(thread) => blue_done(app, duel, cx),
        _ => {}
    }
}

fn busy(id: Uuid, on: bool, cx: &mut App) {
    cx.update_global::<Watching, _>(|w, _| {
        if on {
            w.busy.insert(id);
        } else {
            w.busy.remove(&id);
        }
    });
}

/// The checkpoint before the session's first message (where the duel began).
fn first_checkpoint(session: &ThreadSession) -> Option<String> {
    session
        .items
        .iter()
        .find_map(|item| match &item.content {
            ItemContent::User { checkpoint, .. } => Some(checkpoint.clone()),
            _ => None,
        })
        .flatten()
}

/// The checkpoint before the session's last message.
fn last_checkpoint(session: &ThreadSession) -> Option<String> {
    session
        .items
        .iter()
        .rev()
        .find_map(|item| match &item.content {
            ItemContent::User { checkpoint, .. } => Some(checkpoint.clone()),
            _ => None,
        })
        .flatten()
}

fn notice(app: &Entity<AppState>, thread: ThreadId, text: String, cx: &mut App) {
    if let Some(session) = app.update(cx, |app, cx| app.session(thread, cx)) {
        session.update(cx, |session, cx| session.notice(text, false, cx));
    }
}

fn send(app: &Entity<AppState>, thread: ThreadId, text: String, cx: &mut App) {
    if let Some(session) = app.update(cx, |app, cx| app.session(thread, cx)) {
        session.update(cx, |session, cx| session.submit(Prompt::text(text), cx));
    }
}

/// What a turn of Red's changed: test files kept, anything else put back.
/// Blocking.
fn keep_tests_only(root: &Path, before: &str) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    let now = elyra_git::checkpoint::snapshot(root, "elyra: duel")?;
    let changed = elyra_git::undo::changed_paths(root, before, &now)?;
    let (tests, others): (Vec<String>, Vec<String>) = changed
        .into_iter()
        .partition(|path| crate::keep_test::is_test(path));
    for path in &others {
        put_back(root, before, path)?;
    }
    Ok((tests, others))
}

/// A file as it is in `snapshot` (removed when it isn't there).
fn put_back(root: &Path, snapshot: &str, path: &str) -> std::io::Result<()> {
    let target = root.join(path);
    match elyra_git::undo::content_at(root, snapshot, path) {
        Some(content) => {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(target, content)
        }
        None if target.exists() => std::fs::remove_file(target),
        None => Ok(()),
    }
}

struct RedCheck {
    tests: Vec<String>,
    others: Vec<String>,
    /// The checks passed (None: not run, no tests).
    passed: Option<bool>,
    locked: Option<String>,
}

/// Red's turn ended: tests only, and they must fail; then they go to Blue.
fn red_done(app: &Entity<AppState>, mut duel: Duel, cx: &mut App) {
    let Some(session) = app.update(cx, |app, cx| app.session(duel.red, cx)) else {
        return;
    };
    // Everything Red wrote since the duel began.
    let (before, dir) = {
        let session = session.read(cx);
        (first_checkpoint(session), session.working_dir())
    };
    let command = app
        .read(cx)
        .project(duel.project)
        .and_then(|p| p.check_command.clone())
        .filter(|c| !c.trim().is_empty());
    let (Some(before), Some(command)) = (before, command) else {
        return;
    };
    busy(duel.id, true, cx);
    let refname = format!("refs/elyra/duel/{}/tests", duel.id.simple());
    let job = cx.background_executor().spawn(async move {
        let root = elyra_git::repo_root(&dir)?;
        let (tests, others) = keep_tests_only(&root, &before)?;
        if tests.is_empty() {
            return anyhow::Ok(RedCheck {
                tests,
                others,
                passed: None,
                locked: None,
            });
        }
        let outcome = crate::checks::run(&dir, &command, &Arc::new(AtomicBool::new(false)));
        let locked = (!outcome.passed)
            .then(|| elyra_git::checkpoint::create(&root, &refname, "elyra: duel tests").ok())
            .flatten();
        Ok(RedCheck {
            tests,
            others,
            passed: Some(outcome.passed),
            locked,
        })
    });
    let app = app.clone();
    cx.spawn(async move |cx| {
        let checked = job.await;
        cx.update(|cx| {
            busy(duel.id, false, cx);
            let checked = match checked {
                Ok(checked) => checked,
                Err(err) => {
                    notice(&app, duel.red, format!("The duel couldn't check the tests: {err:#}"), cx);
                    return;
                }
            };
            if !checked.others.is_empty() {
                notice(
                    &app,
                    duel.red,
                    format!(
                        "Put back what isn't a test (Red writes tests only): {}.",
                        checked.others.join(", ")
                    ),
                    cx,
                );
            }
            let retry = match checked.passed {
                None => Some("You didn't add or change any test files. Write the tests for the requirement (only test files).".to_string()),
                Some(true) => Some(format!(
                    "Your tests ({}) pass against the current code, so they don't pin down anything the code doesn't do yet. Make them test what the requirement adds, so they fail now.",
                    checked.tests.join(", ")
                )),
                Some(false) => None,
            };
            if let Some(retry) = retry {
                duel.tries += 1;
                if duel.tries >= MAX_TRIES {
                    duel.phase = Phase::Stopped;
                    save(&app.read(cx).store, &duel);
                    notice(&app, duel.red, format!("The duel stopped: after {MAX_TRIES} tries there are no tests that fail against the current code."), cx);
                } else {
                    save(&app.read(cx).store, &duel);
                    send(&app, duel.red, retry, cx);
                }
                return;
            }
            let Some(locked) = checked.locked else {
                notice(&app, duel.red, "The duel couldn't lock the tests.".into(), cx);
                return;
            };
            duel.tests = checked.tests;
            duel.locked = Some(locked);
            duel.phase = Phase::Implementing;
            match hand_to_blue(&app, &duel, cx) {
                Ok(blue) => {
                    duel.blue = Some(blue);
                    save(&app.read(cx).store, &duel);
                    notice(
                        &app,
                        duel.red,
                        format!(
                            "Your tests fail against the current code, as they should: locked and handed to Blue ({}): {}.",
                            duel.blue_provider.label(),
                            duel.tests.join(", ")
                        ),
                        cx,
                    );
                    watch(&app, blue, cx);
                }
                Err(err) => notice(&app, duel.red, format!("The duel couldn't start Blue: {err}"), cx),
            }
        });
    })
    .detach();
}

/// Blue's thread, in Red's worktree, with the tests locked.
fn hand_to_blue(app: &Entity<AppState>, duel: &Duel, cx: &mut App) -> Result<ThreadId, String> {
    let environment = app
        .read(cx)
        .thread(duel.red)
        .map(|t| t.environment.clone())
        .ok_or("Red's thread is gone")?;
    let thread = app
        .update(cx, |app, cx| app.create_thread(duel.project, cx))
        .map_err(|err| format!("{err:#}"))?;
    let short: String = duel
        .requirement
        .lines()
        .next()
        .unwrap_or("")
        .chars()
        .take(40)
        .collect();
    app.update(cx, |app, cx| {
        app.update_thread(thread.id, |t| t.environment = environment.clone(), cx)
    });
    let session = app
        .update(cx, |app, cx| app.session(thread.id, cx))
        .ok_or("could not open the thread")?;
    let prompt = blue_prompt(&duel.requirement, &duel.tests);
    session.update(cx, |session, cx| {
        session.set_provider(duel.blue_provider, cx);
        session.rename(format!("Blue · code · {short}"), cx);
        session.submit(Prompt::text(prompt), cx);
    });
    Ok(thread.id)
}

/// Blue's turn ended: put back locked tests it changed; green checks end it.
fn blue_done(app: &Entity<AppState>, mut duel: Duel, cx: &mut App) {
    let (Some(blue), Some(locked)) = (duel.blue, duel.locked.clone()) else {
        return;
    };
    let Some(session) = app.update(cx, |app, cx| app.session(blue, cx)) else {
        return;
    };
    let (dir, green) = {
        let session = session.read(cx);
        (session.working_dir(), last_checks_passed(&session.items))
    };
    busy(duel.id, true, cx);
    let tests = duel.tests.clone();
    let job = cx.background_executor().spawn(async move {
        let root = elyra_git::repo_root(&dir)?;
        let mut restored = Vec::new();
        for path in &tests {
            let now = std::fs::read(root.join(path)).ok();
            if now != elyra_git::undo::content_at(&root, &locked, path) {
                put_back(&root, &locked, path)?;
                restored.push(path.clone());
            }
        }
        anyhow::Ok(restored)
    });
    let app = app.clone();
    cx.spawn(async move |cx| {
        let restored = job.await;
        cx.update(|cx| {
            busy(duel.id, false, cx);
            let restored = match restored {
                Ok(restored) => restored,
                Err(err) => {
                    notice(&app, blue, format!("The duel couldn't check the locked tests: {err:#}"), cx);
                    return;
                }
            };
            if !restored.is_empty() {
                duel.restores += 1;
                if duel.restores > MAX_RESTORES {
                    duel.phase = Phase::Stopped;
                    save(&app.read(cx).store, &duel);
                    let text = format!("The duel stopped: Blue changed Red's locked tests {MAX_RESTORES} times. They are back as Red wrote them.");
                    notice(&app, blue, text.clone(), cx);
                    notice(&app, duel.red, text, cx);
                    return;
                }
                save(&app.read(cx).store, &duel);
                notice(&app, blue, format!("Put back Red's locked tests: {}.", restored.join(", ")), cx);
                send(
                    &app,
                    blue,
                    format!(
                        "Elyra put back Red's locked tests you changed ({}). Make the code pass them as they are. If you are sure one is wrong, call challenge_test instead of changing it.",
                        restored.join(", ")
                    ),
                    cx,
                );
                return;
            }
            if green == Some(true) {
                duel.phase = Phase::Done;
                save(&app.read(cx).store, &duel);
                let text = format!(
                    "The duel is won: Blue's code passes Red's tests ({}), and the checks are green.",
                    duel.tests.join(", ")
                );
                notice(&app, blue, text.clone(), cx);
                notice(&app, duel.red, text, cx);
            }
        });
    })
    .detach();
}

/// Whether the checks after the last message passed (None: none ran).
fn last_checks_passed(items: &[elyra_core::TranscriptItem]) -> Option<bool> {
    items
        .iter()
        .rev()
        .take_while(|i| !matches!(i.content, ItemContent::User { .. }))
        .find_map(|i| match i.content {
            ItemContent::Check { passed, .. } => Some(passed),
            _ => None,
        })
}

/// Blue challenges a test: Red answers (and may change it). Returns what to
/// tell Blue. Blocks until Red's turn is over.
pub async fn challenge(
    app: &Entity<AppState>,
    blue: ThreadId,
    file: String,
    test: String,
    reason: String,
    cx: &mut AsyncApp,
) -> Result<String, String> {
    let duel = cx
        .update(|cx| of_thread(&app.read(cx).store, blue))
        .filter(|d| d.blue == Some(blue) && d.phase == Phase::Implementing)
        .ok_or("Only Blue in a duel that is in progress can challenge a test.")?;
    if duel.challenges >= MAX_CHALLENGES {
        return Err(format!(
            "You have challenged {MAX_CHALLENGES} tests already, the most a duel allows. Make the code pass them."
        ));
    }
    let red = cx
        .update(|cx| app.update(cx, |app, cx| app.session(duel.red, cx)))
        .ok_or("Red's thread is gone.")?;
    cx.update(|cx| {
        red.update(cx, |red, cx| {
            red.submit(Prompt::text(challenge_prompt(&file, &test, &reason)), cx)
        })
    });
    // Wait for the message to start, then for Red's turn to end.
    cx.background_executor()
        .timer(std::time::Duration::from_millis(500))
        .await;
    crate::gateway::settle(app, duel.red, std::time::Duration::from_secs(1800), cx).await;
    let (before, dir, reply) = cx.update(|cx| {
        let red = red.read(cx);
        (last_checkpoint(red), red.working_dir(), red.last_reply())
    });
    let before = before.ok_or("Red's answer has no checkpoint.")?;
    let refname = format!("refs/elyra/duel/{}/tests", duel.id.simple());
    let changed = cx
        .background_executor()
        .spawn(async move {
            let root: PathBuf = elyra_git::repo_root(&dir)?;
            let (tests, _others) = keep_tests_only(&root, &before)?;
            let locked = if tests.is_empty() {
                None
            } else {
                Some(elyra_git::checkpoint::create(
                    &root,
                    &refname,
                    "elyra: duel tests",
                )?)
            };
            anyhow::Ok((tests, locked))
        })
        .await
        .map_err(|err| format!("{err:#}"))?;
    let (tests, locked) = changed;
    cx.update(|cx| {
        let store = &app.read(cx).store;
        let Some(mut duel) = of_thread(store, blue) else {
            return;
        };
        duel.challenges += 1;
        if let Some(locked) = locked.clone() {
            duel.locked = Some(locked);
            for test in &tests {
                if !duel.tests.contains(test) {
                    duel.tests.push(test.clone());
                }
            }
        }
        save(store, &duel);
    });
    Ok(format!(
        "Red answered:\n\n{}\n\n{}",
        reply.trim(),
        if tests.is_empty() {
            "Red didn't change the tests: they stay locked as they are.".to_string()
        } else {
            format!(
                "Red changed {}; those are now the locked tests.",
                tests.join(", ")
            )
        }
    ))
}

// ---- starting one -------------------------------------------------------------------

/// The requirement, and which agent writes the tests and which the code.
pub struct DuelForm {
    requirement: Entity<gpui_kit::component::input::TextareaState>,
    available: Vec<ProviderKind>,
    red: Option<ProviderKind>,
    blue: Option<ProviderKind>,
}

impl DuelForm {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let requirement = cx.new(|cx| {
            gpui_kit::component::input::TextareaState::new(window, cx)
                .auto_grow(4, 12)
                .placeholder(
                    "The requirement, e.g. members get 15 % off orders over 500 kr, rounded down",
                )
        });
        let available = crate::preferences::Preferences::global(cx).enabled_providers();
        // Two different agents, when there are two: their blind spots differ.
        let blue = available.first().copied();
        let red = available.get(1).copied().or(blue);
        Self {
            requirement,
            available,
            red,
            blue,
        }
    }
}

impl Render for DuelForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::button::{Button, ButtonVariants as _};
        use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
        use gpui_kit::prelude::FluentBuilder as _;
        let row = |label: &'static str,
                   red: bool,
                   chosen: Option<ProviderKind>,
                   cx: &mut Context<Self>| {
            v_flex()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(label),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .flex_wrap()
                        .children(self.available.iter().map(|&kind| {
                            let on = chosen == Some(kind);
                            Button::new(SharedString::from(format!(
                                "duel-{}-{}",
                                if red { "red" } else { "blue" },
                                kind.as_str()
                            )))
                            .small()
                            .when(on, |this| this.primary())
                            .when(!on, |this| this.outline())
                            .label(kind.label())
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if red {
                                        this.red = Some(kind);
                                    } else {
                                        this.blue = Some(kind);
                                    }
                                    cx.notify();
                                },
                            ))
                        })),
                )
        };
        v_flex()
            .gap_3()
            .child(gpui_kit::component::input::Textarea::new(&self.requirement))
            .child(row("Red writes the tests", true, self.red, cx))
            .child(row("Blue writes the code", false, self.blue, cx))
            .child(
                div()
                    .text_xs()
                    .whitespace_normal()
                    .text_color(cx.theme().muted_foreground)
                    .child("Red's tests must fail against the current code; then Blue gets them, locked, in the same new worktree, until the checks pass. Blue can challenge a test it is sure is wrong. Two different agents catch more."),
            )
    }
}

/// The "New duel" dialog.
pub fn start_dialog(app: Entity<AppState>, project: ProjectId, window: &mut Window, cx: &mut App) {
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    use gpui_kit::component::notification::Notification;
    use gpui_kit::component::{Disableable as _, Sizable as _, WindowExt as _, h_flex};
    let form = cx.new(|cx| DuelForm::new(window, cx));
    let requirement = form.read(cx).requirement.clone();
    requirement.update(cx, |input, cx| input.focus(window, cx));
    window.open_dialog(cx, move |dialog, _, cx| {
        let ready = {
            let form = form.read(cx);
            form.red.is_some()
                && form.blue.is_some()
                && !form.requirement.read(cx).value().trim().is_empty()
        };
        let (form, app) = (form.clone(), app.clone());
        dialog
            .title("Red vs. Blue")
            .w(px(560.))
            .child(form.clone())
            .footer(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("duel-cancel")
                            .small()
                            .label("Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("duel-start")
                            .small()
                            .primary()
                            .label("Start the duel")
                            .disabled(!ready)
                            .on_click(move |_, window, cx| {
                                let (requirement, red, blue) = {
                                    let form = form.read(cx);
                                    (
                                        form.requirement.read(cx).value().to_string(),
                                        form.red,
                                        form.blue,
                                    )
                                };
                                let (Some(red), Some(blue)) = (red, blue) else {
                                    return;
                                };
                                window.close_dialog(cx);
                                match start(&app, project, requirement, red, blue, cx) {
                                    Ok(thread) => {
                                        if let Some(hub) =
                                            cx.try_global::<crate::browser_tools::BrowserHub>()
                                        {
                                            let workspace = hub.workspace.clone();
                                            let _ = workspace.update(cx, |this, cx| {
                                                this.activate(thread, window, cx)
                                            });
                                        }
                                    }
                                    Err(err) => {
                                        window.push_notification(Notification::error(err), cx)
                                    }
                                }
                            }),
                    ),
            )
    });
}

#[cfg(test)]
mod tests {
    use super::{BLUE, Duel, Phase, RED, blue_prompt, last_checks_passed, red_prompt};
    use elyra_core::{ItemContent, ProviderKind, TranscriptItem};

    #[test]
    fn red_writes_tests_and_blue_the_code() {
        let red = red_prompt("Members get 15 % off, rounded down to whole kroner.");
        assert!(red.starts_with(RED));
        assert!(
            red.contains("Members get 15 % off")
                && red.contains("must fail against the current code")
        );
        let blue = blue_prompt(
            "Members get 15 % off.",
            &["tests/Feature/DiscountTest.php".into()],
        );
        assert!(blue.starts_with(BLUE));
        assert!(
            blue.contains("- tests/Feature/DiscountTest.php") && blue.contains("challenge_test")
        );
        let duel = Duel {
            id: uuid::Uuid::new_v4(),
            project: elyra_core::new_id(),
            requirement: "r".into(),
            red: elyra_core::new_id(),
            blue_provider: ProviderKind::Claude,
            blue: None,
            phase: Phase::Specifying,
            tests: vec![],
            locked: None,
            tries: 0,
            restores: 0,
            challenges: 0,
        };
        let json = serde_json::to_string(&vec![duel.clone()]).unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<Duel>>(&json).unwrap(),
            vec![duel]
        );
    }

    #[test]
    fn reads_the_last_checks() {
        let item = |content| TranscriptItem {
            id: elyra_core::new_id(),
            thread_id: elyra_core::new_id(),
            seq: 0,
            content,
            created_at: chrono::Utc::now(),
        };
        let check = |passed| {
            item(ItemContent::Check {
                command: "t".into(),
                passed,
                exit_code: None,
                duration_ms: 0,
                output: String::new(),
            })
        };
        let user = item(ItemContent::User {
            text: "go".into(),
            checkpoint: None,
            attachments: Vec::new(),
        });
        assert_eq!(last_checks_passed(&[check(false), user.clone()]), None);
        assert_eq!(
            last_checks_passed(&[user.clone(), check(false), check(true)]),
            Some(true)
        );
    }
}
