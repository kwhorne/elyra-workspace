//! Browser journeys: a flow an agent walked through in the thread's browser
//! (open a page, click, fill in, press, wait, expect), saved with the project
//! in `.elyra/journeys/<name>.json` and replayed later — on request, or after
//! every turn that changed files (once the user turned that on) — so a change
//! that breaks the flow is caught like a failing test.
//!
//! After every step, the page's layout is measured as well (the page wider
//! than the window, controls outside it or covering each other, text cut
//! off), at the browser's width and at a phone's ([`PHONE_WIDTH`]). In an app
//! Elyra Grove runs, the database queries of the step's requests are counted
//! too. The first replay saves how it looks and how many queries each step
//! makes in `.elyra/journeys/layout/<name>.json`; later replays report only
//! layout problems the page didn't have, and steps whose queries jumped (an
//! N+1 query: 5 became 120), and those fail the journey like a broken step,
//! until fixed or accepted as the new normal.
//!
//! This module is the file format and the words around it; replaying lives in
//! `browser_tools`, which drives the page.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Where a project keeps its journeys.
pub const DIR: &str = ".elyra/journeys";
/// The width pages are also measured at, in points (an iPhone's).
pub const PHONE_WIDTH: f64 = 390.;

/// An element: by CSS selector, by visible text (label, placeholder), or both.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Target {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl Target {
    pub fn describe(&self) -> String {
        match (self.text.as_deref(), self.selector.as_deref()) {
            (Some(text), _) => format!("\u{201c}{text}\u{201d}"),
            (None, Some(selector)) => format!("`{selector}`"),
            (None, None) => "the focused element".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// Load a page (a local address).
    Open(String),
    Click(Target),
    Fill {
        #[serde(flatten)]
        target: Target,
        value: String,
    },
    Press {
        key: String,
        #[serde(flatten)]
        target: Target,
    },
    /// Wait until it is on the page.
    Wait(Target),
    /// It must be on the page now (after a short wait); the journey's verdict.
    Expect(Target),
}

impl Step {
    pub fn describe(&self) -> String {
        match self {
            Step::Open(url) => format!("open {url}"),
            Step::Click(target) => format!("click {}", target.describe()),
            Step::Fill { target, value } => {
                format!("fill in {} with \u{201c}{value}\u{201d}", target.describe())
            }
            Step::Press { key, target } => format!("press {key} in {}", target.describe()),
            Step::Wait(target) => format!("wait for {}", target.describe()),
            Step::Expect(target) => format!("expect {}", target.describe()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Journey {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub steps: Vec<Step>,
    /// Measure the layout at a phone's width too (`"phone": false` turns it off).
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub phone: bool,
}

fn yes() -> bool {
    true
}

fn is_yes(value: &bool) -> bool {
    *value
}

impl Journey {
    /// Saved journeys need somewhere to start and something to check.
    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("a journey needs a name".into());
        }
        if !matches!(self.steps.first(), Some(Step::Open(_))) {
            return Err("a journey starts by opening a page".into());
        }
        if !self.steps.iter().any(|s| matches!(s, Step::Expect(_))) {
            return Err(
                "a journey needs at least one expect step: what must be on the page when it works"
                    .into(),
            );
        }
        Ok(())
    }
}

/// The file a journey is saved in: its name, lowercased, words joined by `-`.
pub fn file_name(name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let slug = slug
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!("{}.json", if slug.is_empty() { "journey" } else { &slug })
}

/// A project's journeys, by file name; unreadable files are reported.
pub fn load_all(dir: &Path) -> Vec<(PathBuf, Result<Journey, String>)> {
    let Ok(entries) = std::fs::read_dir(dir.join(DIR)) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let journey = std::fs::read_to_string(&path)
                .map_err(|err| err.to_string())
                .and_then(|text| serde_json::from_str(&text).map_err(|err| err.to_string()));
            (path, journey)
        })
        .collect()
}

pub fn find(dir: &Path, name: &str) -> Option<Journey> {
    load_all(dir).into_iter().find_map(|(path, journey)| {
        let journey = journey.ok()?;
        (journey.name.eq_ignore_ascii_case(name)
            || path.file_name().is_some_and(|f| *f == *file_name(name)))
        .then_some(journey)
    })
}

/// Write a journey into the project (replacing one of the same name).
pub fn save(dir: &Path, journey: &Journey) -> Result<PathBuf, String> {
    journey.validate()?;
    let folder = dir.join(DIR);
    std::fs::create_dir_all(&folder).map_err(|err| err.to_string())?;
    let path = folder.join(file_name(&journey.name));
    let text = serde_json::to_string_pretty(journey).map_err(|err| err.to_string())?;
    std::fs::write(&path, text + "\n").map_err(|err| err.to_string())?;
    Ok(path)
}

/// Put a recorded or saved address on `origin` (the thread's own site, such
/// as a worktree's), keeping its path.
pub fn rebase(url: &str, origin: Option<&str>) -> String {
    let Some(origin) = origin else {
        return url.to_string();
    };
    let path = url
        .split_once("://")
        .and_then(|(_, rest)| rest.find('/').map(|at| &rest[at..]))
        .unwrap_or("/");
    format!("{}{path}", origin.trim_end_matches('/'))
}

/// `scheme://host[:port]` of a URL.
pub fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split('/').next()?;
    (!host.is_empty()).then(|| format!("{scheme}://{host}"))
}

/// What a replay found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Outcome {
    pub passed: bool,
    pub steps_run: usize,
    /// The step that failed and why.
    pub failure: Option<String>,
    pub duration_ms: u64,
    /// The layout after each step that ran: (step, problems).
    pub layout: Vec<(usize, Vec<Measured>)>,
    /// The same at a phone's width (steps it could be measured after).
    pub phone_layout: Vec<(usize, Vec<Measured>)>,
    /// The database queries of each step's requests (with Grove).
    pub queries: Vec<StepQueries>,
    /// Problems the page didn't have before (filled in by [`guard`]).
    pub layout_new: Vec<elyra_core::LayoutProblem>,
    /// Steps whose queries jumped (filled in by [`guard`]).
    pub query_jumps: Vec<elyra_core::QueryJump>,
    /// The first guarded replay saved the layout, with this many problems.
    pub layout_saved: Option<usize>,
}

/// How many queries a step's requests made, and the one repeated most.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StepQueries {
    pub step: usize,
    pub count: usize,
    pub repeated: Option<(String, usize)>,
}

/// Whether a step's queries went from `before` to `after` the way an N+1
/// query does: at least doubled, and by at least 20.
pub fn jumped(before: usize, after: usize) -> bool {
    after >= before * 2 && after >= before + 20
}

/// A layout problem as agent-act.js measures it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Measured {
    pub key: String,
    pub kind: String,
    pub element: String,
    pub detail: String,
}

/// What agent-act.js `layout()` answered.
pub fn parse_layout(json: &str) -> Vec<Measured> {
    #[derive(Deserialize)]
    struct Answer {
        #[serde(default)]
        issues: Vec<Measured>,
    }
    serde_json::from_str::<Answer>(json)
        .map(|a| a.issues)
        .unwrap_or_default()
}

// ---- the layout guard ----------------------------------------------------------

/// How a journey's pages looked: per step, what it does and its problems' keys.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Baseline {
    steps: Vec<BaselineStep>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct BaselineStep {
    step: String,
    keys: Vec<String>,
    /// The problems at a phone's width (None: not measured there yet).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    phone_keys: Option<Vec<String>>,
    /// How many database queries the step made (None: not counted yet).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    queries: Option<usize>,
}

fn baseline_path(dir: &Path, name: &str) -> PathBuf {
    dir.join(DIR).join("layout").join(file_name(name))
}

fn read_baseline(dir: &Path, name: &str) -> Option<Baseline> {
    serde_json::from_str(&std::fs::read_to_string(baseline_path(dir, name)).ok()?).ok()
}

fn write_baseline(dir: &Path, name: &str, baseline: &Baseline) {
    let path = baseline_path(dir, name);
    let written = path
        .parent()
        .map(std::fs::create_dir_all)
        .transpose()
        .and_then(|_| {
            std::fs::write(
                &path,
                serde_json::to_string_pretty(baseline).unwrap_or_default() + "\n",
            )
        });
    if let Err(err) = written {
        log::warn!("saving the layout of “{name}”: {err:#}");
    }
}

/// Compare a replay with how the journey's pages looked before, and how
/// many queries its steps made: new layout problems (at either width) and
/// queries that jumped fail it. The first time, and for steps that changed,
/// what it found is saved as it is; fewer queries become the new count.
pub fn guard(dir: &Path, journey: &Journey, outcome: &mut Outcome) {
    if !outcome.passed || (outcome.layout.is_empty() && outcome.queries.is_empty()) {
        return;
    }
    let step_text = |index: usize| {
        journey
            .steps
            .get(index)
            .map(Step::describe)
            .unwrap_or_default()
    };
    let saved = read_baseline(dir, &journey.name);
    let mut baseline = saved.clone().unwrap_or_default();
    let mut new = Vec::new();
    let mut jumps = Vec::new();
    let slot = |baseline: &mut Baseline, index: usize| -> bool {
        if baseline.steps.len() <= index {
            baseline.steps.resize(index + 1, BaselineStep::default());
        }
        // A step that does something else now starts over.
        let text = step_text(index);
        let known = &mut baseline.steps[index];
        let fresh = saved.is_none() || known.step != text;
        if fresh {
            *known = BaselineStep {
                step: text,
                ..Default::default()
            };
        }
        fresh
    };
    let mut fresh_steps = std::collections::HashSet::new();
    for index in outcome
        .layout
        .iter()
        .map(|(i, _)| *i)
        .chain(outcome.phone_layout.iter().map(|(i, _)| *i))
        .chain(outcome.queries.iter().map(|q| q.step))
    {
        if !fresh_steps.contains(&index) && slot(&mut baseline, index) {
            fresh_steps.insert(index);
        }
    }
    let problem = |index: usize, m: &Measured, phone: bool| elyra_core::LayoutProblem {
        step: index,
        step_text: step_text(index),
        kind: m.kind.clone(),
        element: m.element.clone(),
        detail: m.detail.clone(),
        key: m.key.clone(),
        phone,
    };
    for (index, measured) in &outcome.layout {
        let keys: Vec<String> = measured.iter().map(|m| m.key.clone()).collect();
        let known = &mut baseline.steps[*index];
        if fresh_steps.contains(index) {
            known.keys = keys;
            continue;
        }
        new.extend(
            measured
                .iter()
                .filter(|m| !known.keys.contains(&m.key))
                .map(|m| problem(*index, m, false)),
        );
    }
    for (index, measured) in &outcome.phone_layout {
        let keys: Vec<String> = measured.iter().map(|m| m.key.clone()).collect();
        let known = &mut baseline.steps[*index];
        match &known.phone_keys {
            // Not seen at a phone's width before: this is how it looks.
            None => known.phone_keys = Some(keys),
            Some(old) => new.extend(
                measured
                    .iter()
                    .filter(|m| !old.contains(&m.key))
                    .map(|m| problem(*index, m, true)),
            ),
        }
    }
    for counted in &outcome.queries {
        let known = &mut baseline.steps[counted.step];
        match known.queries {
            Some(before) if jumped(before, counted.count) => jumps.push(elyra_core::QueryJump {
                step: counted.step,
                step_text: step_text(counted.step),
                before,
                after: counted.count,
                repeated: counted.repeated.as_ref().map(|(shape, _)| shape.clone()),
                times: counted.repeated.as_ref().map(|(_, n)| *n).unwrap_or(0),
            }),
            // Fewer is the new normal; a little more isn't news.
            Some(before) if counted.count >= before => {}
            _ => known.queries = Some(counted.count),
        }
    }
    baseline.steps.truncate(journey.steps.len());
    if saved.is_none() {
        outcome.layout_saved = Some(
            outcome
                .layout
                .iter()
                .chain(&outcome.phone_layout)
                .map(|(_, m)| m.len())
                .sum(),
        );
    }
    if saved.as_ref() != Some(&baseline) {
        write_baseline(dir, &journey.name, &baseline);
    }
    let mut why = Vec::new();
    if !new.is_empty() {
        why.push(format!(
            "the layout changed: {}",
            new.iter()
                .take(5)
                .map(describe_problem)
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    if !jumps.is_empty() {
        why.push(format!(
            "the database queries jumped: {}",
            jumps
                .iter()
                .map(describe_jump)
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    if !why.is_empty() {
        outcome.passed = false;
        outcome.failure = Some(why.join("; and "));
        outcome.layout_new = new;
        outcome.query_jumps = jumps;
    }
}

/// Accept the problems, and the query counts, as how the pages are now.
pub fn accept_layout(
    dir: &Path,
    name: &str,
    problems: &[elyra_core::LayoutProblem],
    jumps: &[elyra_core::QueryJump],
) {
    let mut baseline = read_baseline(dir, name).unwrap_or_default();
    let mut slot = |step: usize, text: &str| {
        if baseline.steps.len() <= step {
            baseline.steps.resize(step + 1, BaselineStep::default());
        }
        let known = &mut baseline.steps[step];
        if known.step.is_empty() {
            known.step = text.to_string();
        }
        step
    };
    let mut accepted: Vec<(usize, &elyra_core::LayoutProblem)> = Vec::new();
    for problem in problems {
        accepted.push((slot(problem.step, &problem.step_text), problem));
    }
    let jumps: Vec<(usize, usize)> = jumps
        .iter()
        .map(|jump| (slot(jump.step, &jump.step_text), jump.after))
        .collect();
    for (step, problem) in accepted {
        let known = &mut baseline.steps[step];
        let keys = if problem.phone {
            known.phone_keys.get_or_insert_with(Vec::new)
        } else {
            &mut known.keys
        };
        if !keys.contains(&problem.key) {
            keys.push(problem.key.clone());
        }
    }
    for (step, after) in jumps {
        baseline.steps[step].queries = Some(after);
    }
    write_baseline(dir, name, &baseline);
}

/// A jump in words.
pub fn describe_jump(jump: &elyra_core::QueryJump) -> String {
    let mut text = format!(
        "after \u{201c}{}\u{201d}, {} queries instead of {}",
        jump.step_text, jump.after, jump.before
    );
    if let Some(repeated) = &jump.repeated {
        text.push_str(&format!(" (`{repeated}` {}×)", jump.times));
    }
    text
}

/// One problem in words.
pub fn describe_problem(problem: &elyra_core::LayoutProblem) -> String {
    let what = match problem.kind.as_str() {
        "overflow" => "the page is too wide",
        "offscreen" => "outside the window",
        "overlap" => "overlapping",
        "clipped" => "text cut off",
        other => other,
    };
    format!(
        "after “{}”{}, {what}: {} ({})",
        problem.step_text,
        if problem.phone {
            format!(" at {PHONE_WIDTH} px wide")
        } else {
            String::new()
        },
        problem.element,
        problem.detail
    )
}

/// The message that sends a failed journey back to the agent.
pub fn fix_prompt(name: &str, outcome: &Outcome, attempt: u32, attempts: u32) -> String {
    if !outcome.query_jumps.is_empty() {
        let jumps: Vec<String> = outcome
            .query_jumps
            .iter()
            .map(|jump| format!("- {}", describe_jump(jump)))
            .collect();
        let layout: String = outcome
            .layout_new
            .iter()
            .map(|p| format!("\n- {}", describe_problem(p)))
            .collect();
        return format!(
            "The browser journey \u{201c}{name}\u{201d} still works, but your changes made its steps run many more database queries than before:\n{}\n\nA query repeated per row is an N+1 query: load the relation up front (eager loading, a join or one query for all of them). Then finish your turn; the journeys run again (automatic fix {attempt} of {attempts}). If the new count is intended, say so: the user can accept it.{}",
            jumps.join("\n"),
            if layout.is_empty() {
                String::new()
            } else {
                format!("\n\nIts pages' layout changed too:{layout}")
            }
        );
    }
    if !outcome.layout_new.is_empty() {
        let problems: Vec<String> = outcome
            .layout_new
            .iter()
            .map(|p| format!("- {}", describe_problem(p)))
            .collect();
        return format!(
            "The browser journey \u{201c}{name}\u{201d} still works, but your changes broke its pages' layout (these weren't there before):\n{}\n\nFix the cause in the CSS or markup, then finish your turn; the journeys run again (automatic fix {attempt} of {attempts}). If the new layout is intended, say so: the user can accept it.",
            problems.join("\n")
        );
    }
    format!(
        "The browser journey \u{201c}{name}\u{201d} failed after your changes, at step {}: {}. \
         It is a saved flow in .elyra/journeys that worked before. Fix the cause in the code \
         (change the journey only if the flow itself was meant to change), then finish your \
         turn; the journeys run again (automatic fix {attempt} of {attempts}).",
        outcome.steps_run + 1,
        outcome.failure.as_deref().unwrap_or("it failed")
    )
}

// ---- replaying after each turn --------------------------------------------------

fn auto_key(project: elyra_core::ProjectId) -> String {
    format!("journeys_after_turn:{project}")
}

/// Whether the project replays its journeys after every turn that changed files.
pub fn replay_after_turn(store: &elyra_core::Store, project: elyra_core::ProjectId) -> bool {
    store.setting(&auto_key(project)).ok().flatten().as_deref() == Some("1")
}

pub fn set_replay_after_turn(store: &elyra_core::Store, project: elyra_core::ProjectId, on: bool) {
    if let Err(err) = store.set_setting(&auto_key(project), if on { "1" } else { "" }) {
        log::warn!("saving the journey setting: {err:#}");
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn the_layout_guard_reports_only_what_changed() {
        use super::{Measured, Outcome, Step, Target, guard as guard_layout};
        let dir = std::env::temp_dir().join(format!("elyra-layout-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let journey = super::Journey {
            name: "Checkout".into(),
            description: String::new(),
            steps: vec![
                Step::Open("http://shop.test/cart".into()),
                Step::Expect(Target {
                    selector: None,
                    text: Some("Pay".into()),
                }),
            ],
            phone: true,
        };
        let measured = |key: &str| Measured {
            key: key.into(),
            kind: key.split(':').next().unwrap().into(),
            element: key.into(),
            detail: "detail".into(),
        };
        let replay = |layout: Vec<(usize, Vec<Measured>)>| Outcome {
            passed: true,
            steps_run: 2,
            layout,
            ..Default::default()
        };
        // The first replay saves the layout as it is.
        let mut first = replay(vec![(0, vec![measured("clipped:td.price")]), (1, vec![])]);
        guard_layout(&dir, &journey, &mut first);
        assert!(first.passed && first.layout_saved == Some(1));
        // Same again: fine.
        let mut same = replay(vec![(0, vec![measured("clipped:td.price")]), (1, vec![])]);
        guard_layout(&dir, &journey, &mut same);
        assert!(same.passed && same.layout_saved.is_none());
        // A new problem fails it.
        let mut broken = replay(vec![
            (0, vec![measured("clipped:td.price")]),
            (1, vec![measured("overlap:button.pay")]),
        ]);
        guard_layout(&dir, &journey, &mut broken);
        assert!(!broken.passed);
        assert_eq!(broken.layout_new.len(), 1);
        assert!(
            broken
                .failure
                .unwrap()
                .starts_with("the layout changed: after “expect “Pay””, overlapping")
        );
        assert!(
            super::fix_prompt(
                "Checkout",
                &Outcome {
                    layout_new: broken.layout_new.clone(),
                    ..Default::default()
                },
                1,
                2
            )
            .contains("broke its pages' layout")
        );
        // Accepted, it isn't reported again.
        super::accept_layout(&dir, "Checkout", &broken.layout_new, &[]);
        let mut again = replay(vec![
            (0, vec![measured("clipped:td.price")]),
            (1, vec![measured("overlap:button.pay")]),
        ]);
        guard_layout(&dir, &journey, &mut again);
        assert!(again.passed);
        // The baseline lives beside the journeys, not among them.
        assert!(dir.join(".elyra/journeys/layout/checkout.json").is_file());
        assert!(super::load_all(&dir).is_empty());

        // At a phone's width: the first time is saved (also for a baseline
        // from before), then only what's new counts.
        let phone = |layout: Vec<(usize, Vec<Measured>)>| Outcome {
            passed: true,
            steps_run: 2,
            layout: vec![
                (0, vec![measured("clipped:td.price")]),
                (1, vec![measured("overlap:button.pay")]),
            ],
            phone_layout: layout,
            ..Default::default()
        };
        let mut first = phone(vec![(0, vec![measured("overflow:table.cart")])]);
        guard_layout(&dir, &journey, &mut first);
        assert!(first.passed, "{:?}", first.failure);
        let mut narrow = phone(vec![(
            0,
            vec![
                measured("overflow:table.cart"),
                measured("offscreen:button.pay"),
            ],
        )]);
        guard_layout(&dir, &journey, &mut narrow);
        assert!(!narrow.passed && narrow.layout_new.len() == 1 && narrow.layout_new[0].phone);
        assert!(
            narrow
                .failure
                .as_deref()
                .unwrap()
                .contains("after “open http://shop.test/cart” at 390 px wide, outside the window"),
            "{:?}",
            narrow.failure
        );
        super::accept_layout(&dir, "Checkout", &narrow.layout_new, &[]);
        let mut accepted = phone(vec![(
            0,
            vec![
                measured("overflow:table.cart"),
                measured("offscreen:button.pay"),
            ],
        )]);
        guard_layout(&dir, &journey, &mut accepted);
        assert!(
            accepted.passed,
            "accepted at the phone's width: {:?}",
            accepted.failure
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_step_whose_queries_jump_fails() {
        use super::{Outcome, Step, StepQueries, Target, accept_layout, fix_prompt, guard, jumped};
        assert!(jumped(5, 120) && jumped(0, 20) && jumped(100, 250));
        assert!(!jumped(5, 15) && !jumped(40, 70) && !jumped(0, 19));
        let dir = std::env::temp_dir().join(format!("elyra-query-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let journey = super::Journey {
            name: "Orders".into(),
            description: String::new(),
            steps: vec![
                Step::Open("http://shop.test/orders".into()),
                Step::Expect(Target {
                    selector: None,
                    text: Some("Orders".into()),
                }),
            ],
            phone: false,
        };
        let replay = |count: usize, repeated: Option<(&str, usize)>| Outcome {
            passed: true,
            steps_run: 2,
            queries: vec![StepQueries {
                step: 0,
                count,
                repeated: repeated.map(|(shape, n)| (shape.to_string(), n)),
            }],
            ..Default::default()
        };
        let mut first = replay(5, None);
        guard(&dir, &journey, &mut first);
        assert!(first.passed);
        let mut more = replay(9, None);
        guard(&dir, &journey, &mut more);
        assert!(more.passed, "a few more isn't news");
        let mut n_plus_one = replay(120, Some(("select * from customers where id = ?", 115)));
        guard(&dir, &journey, &mut n_plus_one);
        assert!(!n_plus_one.passed);
        assert_eq!(
            (
                n_plus_one.query_jumps[0].before,
                n_plus_one.query_jumps[0].after
            ),
            (5, 120)
        );
        assert_eq!(
            n_plus_one.failure.as_deref(),
            Some(
                "the database queries jumped: after “open http://shop.test/orders”, 120 queries instead of 5 (`select * from customers where id = ?` 115×)"
            )
        );
        let prompt = fix_prompt("Orders", &n_plus_one, 1, 2);
        assert!(
            prompt.contains("N+1") && prompt.contains("115×"),
            "{prompt}"
        );
        // Fewer becomes the new count; an accepted jump does too.
        let mut fewer = replay(3, None);
        guard(&dir, &journey, &mut fewer);
        let mut back = replay(40, None);
        guard(&dir, &journey, &mut back);
        assert!(!back.passed, "measured against 3 now");
        accept_layout(&dir, "Orders", &[], &back.query_jumps);
        let mut accepted = replay(40, None);
        guard(&dir, &journey, &mut accepted);
        assert!(accepted.passed);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    use super::{Journey, Step, Target, file_name, load_all, rebase, save};

    fn text(text: &str) -> Target {
        Target {
            text: Some(text.into()),
            selector: None,
        }
    }

    #[test]
    fn reads_and_writes_the_file_format() {
        let json = r##"{
            "name": "Checkout with a discount code",
            "steps": [
                {"open": "http://shop.test/products/1"},
                {"click": {"text": "Add to cart"}},
                {"fill": {"text": "Discount code", "value": "SUMMER"}},
                {"press": {"key": "Enter", "selector": "#code"}},
                {"wait": {"text": "Total"}},
                {"expect": {"text": "Total: 90,00"}}
            ]
        }"##;
        let journey: Journey = serde_json::from_str(json).unwrap();
        assert_eq!(journey.steps.len(), 6);
        assert_eq!(
            journey.steps[2],
            Step::Fill {
                target: text("Discount code"),
                value: "SUMMER".into()
            }
        );
        assert_eq!(journey.steps[3].describe(), "press Enter in `#code`");
        assert!(journey.validate().is_ok());

        let dir = std::env::temp_dir().join(format!("elyra-journeys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = save(&dir, &journey).unwrap();
        assert!(path.ends_with(".elyra/journeys/checkout-with-a-discount-code.json"));
        let loaded = load_all(&dir);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].1.as_ref().unwrap(), &journey);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn refuses_journeys_that_prove_nothing() {
        let mut journey = Journey {
            name: "x".into(),
            description: String::new(),
            steps: vec![Step::Click(text("Go"))],
            phone: true,
        };
        assert!(journey.validate().unwrap_err().contains("opening a page"));
        journey.steps.insert(0, Step::Open("http://a.test/".into()));
        assert!(journey.validate().unwrap_err().contains("expect"));
        assert_eq!(file_name("Æ, ø & å!"), "æ-ø-å.json");
    }

    #[test]
    fn replays_on_the_thread_s_own_site() {
        assert_eq!(
            rebase(
                "http://shop.test/cart?x=1",
                Some("http://fix-cart.shop.test")
            ),
            "http://fix-cart.shop.test/cart?x=1"
        );
        assert_eq!(
            rebase("http://localhost:5173", Some("http://localhost:5174")),
            "http://localhost:5174/"
        );
        assert_eq!(rebase("http://shop.test/a", None), "http://shop.test/a");
    }
}
