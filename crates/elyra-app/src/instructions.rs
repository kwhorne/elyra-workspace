//! What a branch does, by the instructions behind it: the lines the branch
//! adds (commits since it left the default branch, and what isn't committed
//! yet), grouped by the message that had an agent write them. Built from the
//! line record Elyra keeps after every turn, so it works for every agent and
//! across threads, also when an agent committed on its own. Lines no agent in
//! Elyra wrote form their own group. The same groups make a pull request
//! description without asking a model.

use chrono::{DateTime, Utc};
use elyra_core::{ItemContent, ItemId, ProjectId, Store, ThreadId};
use std::collections::HashMap;
use std::path::Path;

/// Where a line came from: the turn (thread, user message) and whether a
/// person has read it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Origin {
    pub thread: ThreadId,
    pub item: ItemId,
    pub at: DateTime<Utc>,
    pub read: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Group {
    /// None: lines no agent in Elyra Workspace is recorded as writing.
    pub turn: Option<(ThreadId, ItemId)>,
    /// The message, its thread's title and agent, and when.
    pub instruction: String,
    pub thread: String,
    pub agent: String,
    pub at: Option<DateTime<Utc>>,
    /// Lines added per file, most first.
    pub files: Vec<(String, usize)>,
    pub lines: usize,
    /// Lines nobody has marked as read.
    pub unread: usize,
    /// Whether the checks passed after the turn (and its automatic fixes).
    pub checks: Option<bool>,
    /// What the turn cost.
    pub cost: Option<f64>,
    /// What the agent said at the end of the turn.
    pub reply: String,
    /// The lines themselves, per file in the order of the files above:
    /// (line number in the file now, text, whether a person has read it).
    pub detail: Vec<(String, Vec<Line>)>,
}

/// A line the branch adds, where it is now.
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub number: usize,
    pub text: String,
    pub read: bool,
    pub hash: i64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Overview {
    /// The branch it is compared with (e.g. `origin/main`).
    pub base: String,
    /// In the order they were asked for; lines from outside Elyra last.
    pub groups: Vec<Group>,
}

/// Group the lines a branch adds (path, origin) by the turn that wrote them.
pub fn group(lines: &[(String, Option<Origin>)]) -> Vec<Group> {
    let mut groups: Vec<Group> = Vec::new();
    let mut files: Vec<HashMap<String, usize>> = Vec::new();
    for (path, origin) in lines {
        let turn = origin.map(|o| (o.thread, o.item));
        let index = match groups.iter().position(|g| g.turn == turn) {
            Some(index) => index,
            None => {
                groups.push(Group {
                    turn,
                    at: origin.map(|o| o.at),
                    ..Default::default()
                });
                files.push(HashMap::new());
                groups.len() - 1
            }
        };
        let group = &mut groups[index];
        group.lines += 1;
        if origin.is_some_and(|o| !o.read) {
            group.unread += 1;
        }
        *files[index].entry(path.clone()).or_default() += 1;
    }
    for (group, files) in groups.iter_mut().zip(files) {
        let mut files: Vec<(String, usize)> = files.into_iter().collect();
        files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        group.files = files;
    }
    groups.sort_by_key(|g| (g.turn.is_none(), g.at));
    groups
}

/// What the branch in `root` does, read from git and (on a connection of its
/// own) the store at `db`. Blocking: run it in the background.
pub fn load(root: &Path, project: ProjectId, db: &Path) -> anyhow::Result<Overview> {
    let (base, sha) = elyra_git::branch_base(root)
        .ok_or_else(|| anyhow::anyhow!("no main or master branch to compare with"))?;
    let added = elyra_git::checkpoint::added_lines(root, &sha)?;
    let mut added_by_file: HashMap<String, Vec<String>> = HashMap::new();
    for (path, text) in &added {
        added_by_file
            .entry(path.clone())
            .or_default()
            .push(text.clone());
    }
    let store = Store::open_read_only(db)?;
    // The newest turn that wrote each line of each file.
    let mut known: HashMap<String, HashMap<i64, Origin>> = HashMap::new();
    let mut lines = Vec::new();
    for (path, text) in added {
        let Some(hash) = elyra_core::line_hash(&text) else {
            continue;
        };
        if let std::collections::hash_map::Entry::Vacant(slot) = known.entry(path.clone()) {
            let origins = store
                .origins_in_file(project, &path)?
                .into_iter()
                .map(|(hash, thread, item, at, read)| {
                    (
                        hash,
                        Origin {
                            thread,
                            item,
                            at,
                            read,
                        },
                    )
                })
                .collect();
            slot.insert(origins);
        }
        let origin = known[&path].get(&hash).copied();
        lines.push((path, origin));
    }
    let mut groups = group(&lines);
    attach_lines(root, &added_by_file, &known, &mut groups);
    let mut transcripts: HashMap<ThreadId, Vec<elyra_core::TranscriptItem>> = HashMap::new();
    for group in &mut groups {
        let Some((thread, item)) = group.turn else {
            continue;
        };
        if let Some(info) = store.thread(thread)? {
            group.thread = info.title;
            group.agent = info.provider.label().to_string();
        }
        let items = match transcripts.entry(thread) {
            std::collections::hash_map::Entry::Occupied(known) => known.into_mut(),
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(store.transcript(thread)?)
            }
        };
        describe_turn(group, items, item);
    }
    Ok(Overview { base, groups })
}

/// Find each group's lines in the files as they are now: the lines the
/// branch adds, in file order, with their numbers.
fn attach_lines(
    root: &Path,
    added: &HashMap<String, Vec<String>>,
    known: &HashMap<String, HashMap<i64, Origin>>,
    groups: &mut [Group],
) {
    let mut paths: Vec<&String> = added.keys().collect();
    paths.sort();
    for path in paths {
        let Ok(content) = std::fs::read_to_string(root.join(path)) else {
            continue;
        };
        // How many times each text was added (a line can repeat).
        let mut pending: HashMap<&str, usize> = HashMap::new();
        for text in &added[path] {
            *pending.entry(text.as_str()).or_default() += 1;
        }
        for (index, text) in content.lines().enumerate() {
            let Some(left) = pending.get_mut(text).filter(|n| **n > 0) else {
                continue;
            };
            *left -= 1;
            let Some(hash) = elyra_core::line_hash(text) else {
                continue;
            };
            let origin = known.get(path).and_then(|o| o.get(&hash)).copied();
            let turn = origin.map(|o| (o.thread, o.item));
            let Some(group) = groups.iter_mut().find(|g| g.turn == turn) else {
                continue;
            };
            let line = Line {
                number: index + 1,
                text: text.to_string(),
                read: origin.is_some_and(|o| o.read),
                hash,
            };
            match group.detail.iter_mut().find(|(p, _)| p == path) {
                Some((_, lines)) => lines.push(line),
                None => group.detail.push((path.clone(), vec![line])),
            }
        }
    }
}

/// The message, and the checks and cost of the turn it started (through the
/// automatic fixes that followed, up to the next message from the user).
fn describe_turn(group: &mut Group, items: &[elyra_core::TranscriptItem], item: ItemId) {
    let Some(start) = items.iter().position(|i| i.id == item) else {
        return;
    };
    if let ItemContent::User { text, .. } = &items[start].content {
        group.instruction = text.trim().to_string();
    }
    let mut cost = None;
    for later in &items[start + 1..] {
        match &later.content {
            ItemContent::User { text, .. } if crate::checks::fix_prompt_label(text).is_none() => {
                break;
            }
            ItemContent::Check { passed, .. } => group.checks = Some(*passed),
            ItemContent::TurnSummary {
                cost_usd: Some(spent),
                ..
            } => *cost.get_or_insert(0.0) += spent,
            ItemContent::Assistant {
                text,
                parent_tool_use_id: None,
            } if !text.trim().is_empty() => group.reply = text.trim().to_string(),
            _ => {}
        }
    }
    group.cost = cost;
}

/// A message's first line, at most `max` characters.
fn headline(text: &str, max: usize) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if line.chars().count() > max {
        format!("{}…", line.chars().take(max - 1).collect::<String>())
    } else {
        line.to_string()
    }
}

/// The groups as Markdown, one section per instruction.
pub fn markdown(overview: &Overview) -> String {
    let mut out = String::new();
    for group in &overview.groups {
        match group.turn {
            Some(_) => {
                out.push_str(&format!("### {}\n\n", headline(&group.instruction, 100)));
                let mut meta = vec![group.thread.clone(), group.agent.clone()];
                if let Some(at) = group.at {
                    meta.push(at.format("%Y-%m-%d").to_string());
                }
                match group.checks {
                    Some(true) => meta.push("checks passed".into()),
                    Some(false) => meta.push("checks failed".into()),
                    None => {}
                }
                meta.retain(|m| !m.is_empty());
                out.push_str(&format!("{}\n\n", meta.join(" · ")));
                let rest: Vec<&str> = group.instruction.lines().skip(1).collect();
                if rest.iter().any(|l| !l.trim().is_empty()) && group.instruction.len() < 1200 {
                    for line in group.instruction.lines() {
                        out.push_str(&format!("> {line}\n"));
                    }
                    out.push('\n');
                }
            }
            None => out.push_str("### Not written by an agent in Elyra Workspace\n\n"),
        }
        for (path, lines) in group.files.iter().take(20) {
            out.push_str(&format!("- `{path}` (+{lines})\n"));
        }
        if group.files.len() > 20 {
            out.push_str(&format!("- and {} more files\n", group.files.len() - 20));
        }
        out.push('\n');
    }
    out
}

/// A pull request title and description from the groups: the instruction
/// behind the most lines is the title.
pub fn pull_request(overview: &Overview, issue: Option<&str>) -> (String, String) {
    let main = overview
        .groups
        .iter()
        .filter(|g| g.turn.is_some())
        .max_by_key(|g| g.lines);
    let title = main
        .map(|g| headline(&g.instruction, 72))
        .unwrap_or_else(|| "Changes".into());
    let mut body = String::from("## What was asked\n\n");
    body.push_str(&markdown(overview));
    let checked: Vec<&Group> = overview
        .groups
        .iter()
        .filter(|g| g.checks.is_some())
        .collect();
    if !checked.is_empty() {
        let passed = checked.iter().filter(|g| g.checks == Some(true)).count();
        body.push_str(&format!(
            "## Checks\n\nThe project's checks passed after {passed} of {} instructions.\n\n",
            checked.len()
        ));
    }
    body.push_str(&format!(
        "Compared with `{}`. Written from the record Elyra Workspace keeps of which instruction wrote which lines.\n",
        overview.base
    ));
    if let Some(issue) = issue {
        body.push_str(&format!("\nFélagi: {issue}\n"));
    }
    (title, body)
}

#[cfg(test)]
mod tests {
    use super::{Group, Origin, Overview, attach_lines, group, markdown, pull_request};
    use chrono::{Duration, Utc};
    use std::collections::HashMap;

    #[test]
    fn finds_each_instructions_lines_where_they_are_now() {
        let dir = std::env::temp_dir().join(format!("elyra-lines-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("cart.rs"),
            "fn total() {}\nlet a = discount(1);\nunchanged line\nlet b = shipping(2);\n",
        )
        .unwrap();
        let (thread, first, second) = (
            elyra_core::new_id(),
            elyra_core::new_id(),
            elyra_core::new_id(),
        );
        let origin = |item, read| Origin {
            thread,
            item,
            at: Utc::now(),
            read,
        };
        let hash = |text: &str| elyra_core::line_hash(text).unwrap();
        let known = HashMap::from([(
            "cart.rs".to_string(),
            HashMap::from([
                (hash("let a = discount(1);"), origin(first, true)),
                (hash("let b = shipping(2);"), origin(second, false)),
            ]),
        )]);
        let added = HashMap::from([(
            "cart.rs".to_string(),
            vec![
                "let a = discount(1);".to_string(),
                "let b = shipping(2);".to_string(),
            ],
        )]);
        let mut groups = vec![
            Group {
                turn: Some((thread, first)),
                ..Default::default()
            },
            Group {
                turn: Some((thread, second)),
                ..Default::default()
            },
        ];
        attach_lines(&dir, &added, &known, &mut groups);
        let lines = |group: &Group| {
            group.detail[0]
                .1
                .iter()
                .map(|l| (l.number, l.read))
                .collect::<Vec<_>>()
        };
        assert_eq!(lines(&groups[0]), [(2, true)]);
        assert_eq!(
            lines(&groups[1]),
            [(4, false)],
            "the unchanged line isn't added"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn groups_lines_by_the_instruction_behind_them() {
        let (thread, first, second) = (
            elyra_core::new_id(),
            elyra_core::new_id(),
            elyra_core::new_id(),
        );
        let early = Utc::now() - Duration::hours(1);
        let origin = |item, at, read| {
            Some(Origin {
                thread,
                item,
                at,
                read,
            })
        };
        let lines = vec![
            ("README.md".to_string(), None),
            ("src/cart.rs".to_string(), origin(second, Utc::now(), false)),
            ("src/cart.rs".to_string(), origin(first, early, true)),
            ("src/cart.rs".to_string(), origin(first, early, false)),
            ("src/totals.rs".to_string(), origin(first, early, true)),
        ];
        let groups = group(&lines);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].turn, Some((thread, first)), "oldest first");
        assert_eq!(
            (groups[0].lines, groups[0].unread),
            (3, 1),
            "{:?}",
            groups[0]
        );
        assert_eq!(
            groups[0].files,
            [
                ("src/cart.rs".to_string(), 2),
                ("src/totals.rs".to_string(), 1)
            ]
        );
        assert_eq!(groups[1].turn, Some((thread, second)));
        assert_eq!(groups[2].turn, None, "outside Elyra last");

        let named = |mut group: Group, instruction: &str, checks| {
            group.instruction = instruction.into();
            group.thread = "Cart".into();
            group.agent = "Claude Code".into();
            group.checks = checks;
            group
        };
        let overview = Overview {
            base: "origin/main".into(),
            groups: vec![
                named(
                    groups[0].clone(),
                    "Round totals after the discount\nAnd add a test.",
                    Some(true),
                ),
                named(groups[1].clone(), "Rename total()", None),
                groups[2].clone(),
            ],
        };
        let text = markdown(&overview);
        assert!(text.starts_with("### Round totals after the discount\n\nCart · Claude Code · "));
        assert!(
            text.contains(
                "checks passed\n\n> Round totals after the discount\n> And add a test.\n"
            )
        );
        assert!(text.contains("- `src/cart.rs` (+2)\n- `src/totals.rs` (+1)\n"));
        assert!(
            text.contains("### Not written by an agent in Elyra Workspace\n\n- `README.md` (+1)")
        );
        let (title, body) = pull_request(&overview, Some("SHOP-12"));
        assert_eq!(
            title, "Round totals after the discount",
            "the instruction behind most lines"
        );
        assert!(body.starts_with("## What was asked\n\n### Round totals"));
        assert!(body.contains("The project's checks passed after 1 of 1 instructions."));
        assert!(body.ends_with("Félagi: SHOP-12\n"));
    }
}
