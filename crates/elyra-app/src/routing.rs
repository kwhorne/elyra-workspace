//! Smart routing: which agent and model to give a message to, from how they
//! did on the same part of your code. Every instruction's outcome (checks
//! green on the first try, cost; see `leaderboard`) is tied to the folders its
//! turns wrote in (the line record), so the numbers are per area: an agent
//! that does well in the Rust core may not in the CSS. A new thread's first
//! message is matched to areas by the paths and symbols it names; when one
//! agent has clearly done better there, the thread suggests it (or picks it,
//! with Settings → Pick the agent from your stats).

use elyra_core::{ItemContent, ProjectId, ProviderKind, Store, ThreadId};
use std::collections::{BTreeSet, HashMap};
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Fewer checked instructions in the area than this, and an agent isn't
/// compared there.
pub const MIN_CHECKED: usize = 3;

/// A folder of the project, two levels deep (`app/Http`, `crates/elyra-app`).
pub fn area(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.len() {
        0 | 1 => "the project's root".into(),
        2 => parts[0].into(),
        _ => format!("{}/{}", parts[0], parts[1]),
    }
}

/// One instruction: who did it, where, and how it went.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub provider: ProviderKind,
    pub model: Option<String>,
    pub areas: BTreeSet<String>,
    pub outcome: crate::leaderboard::Outcome,
}

/// Every instruction of the project's threads (side chats left out).
/// Blocking.
pub fn load(
    db: &Path,
    project: ProjectId,
    threads: &[(ThreadId, ProviderKind, Option<String>)],
) -> anyhow::Result<Vec<Record>> {
    let store = Store::open_read_only(db)?;
    let paths = store.instruction_paths(project)?;
    let mut records = Vec::new();
    for (thread, provider, model) in threads {
        let items = store.transcript(*thread)?;
        let outcomes = crate::leaderboard::outcomes(&items);
        // The messages the outcomes belong to, in the same order.
        let messages = items.iter().filter_map(|item| match &item.content {
            ItemContent::User { text, .. } if crate::checks::fix_prompt_label(text).is_none() => {
                Some(item.id)
            }
            _ => None,
        });
        for (item, outcome) in messages.zip(outcomes) {
            let areas: BTreeSet<String> = paths
                .get(&(*thread, item))
                .into_iter()
                .flatten()
                .map(|p| area(p))
                .collect();
            if areas.is_empty() {
                continue;
            }
            records.push(Record {
                provider: *provider,
                model: model.clone(),
                areas,
                outcome,
            });
        }
    }
    Ok(records)
}

/// What to suggest.
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub provider: ProviderKind,
    pub model: Option<String>,
    /// Why, in a line.
    pub reason: String,
}

#[derive(Default)]
struct Tally {
    checked: usize,
    first_try: usize,
    instructions: usize,
    cost: f64,
}

impl Tally {
    fn rate(&self) -> f64 {
        self.first_try as f64 / self.checked.max(1) as f64
    }
}

fn label(provider: ProviderKind, model: &Option<String>) -> String {
    match model {
        Some(model) => format!("{} · {model}", provider.label()),
        None => provider.label().to_string(),
    }
}

/// The agent that did clearly best in `areas`, if the numbers say so: at
/// least two agents with enough checked instructions there, and the best
/// greener on the first try than the next (or as green and much cheaper).
pub fn recommend(records: &[Record], areas: &BTreeSet<String>) -> Option<Pick> {
    if areas.is_empty() {
        return None;
    }
    let mut tallies: HashMap<(ProviderKind, Option<String>), Tally> = HashMap::new();
    for record in records.iter().filter(|r| !r.areas.is_disjoint(areas)) {
        let tally = tallies
            .entry((record.provider, record.model.clone()))
            .or_default();
        tally.instructions += 1;
        tally.cost += record.outcome.cost;
        if let Some(first_try) = record.outcome.first_try {
            tally.checked += 1;
            tally.first_try += usize::from(first_try);
        }
    }
    let mut compared: Vec<((ProviderKind, Option<String>), Tally)> = tallies
        .into_iter()
        .filter(|(_, t)| t.checked >= MIN_CHECKED)
        .collect();
    if compared.len() < 2 {
        return None;
    }
    let cost_each = |t: &Tally| t.cost / t.instructions.max(1) as f64;
    compared.sort_by(|(_, a), (_, b)| {
        b.rate()
            .total_cmp(&a.rate())
            .then_with(|| cost_each(a).total_cmp(&cost_each(b)))
    });
    let ((provider, model), best) = &compared[0];
    let (next_key, next) = &compared[1];
    let clearly = best.rate() > next.rate() + 0.1
        || (best.rate() >= next.rate() && cost_each(best) < cost_each(next) * 0.6);
    if !clearly {
        return None;
    }
    let mut where_: Vec<&String> = areas.iter().collect();
    where_.truncate(3);
    let where_ = where_
        .iter()
        .map(|a| a.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    Some(Pick {
        provider: *provider,
        model: model.clone(),
        reason: format!(
            "In {where_}, {} has been green on the first try in {} of {} instructions ({} each); {} {} of {} ({} each).",
            label(*provider, model),
            best.first_try,
            best.checked,
            crate::thread_session::format_usd(cost_each(best)),
            label(next_key.0, &next_key.1),
            next.first_try,
            next.checked,
            crate::thread_session::format_usd(cost_each(next)),
        ),
    })
}

/// The areas a message is about: paths it names that are in the project,
/// and where the symbols it names are defined. Blocking.
pub fn areas_for(dir: &Path, message: &str) -> BTreeSet<String> {
    let mut areas = BTreeSet::new();
    for word in message
        .split(|c: char| c.is_whitespace() || matches!(c, '`' | '"' | '\'' | '(' | ')' | ','))
    {
        let word = word.trim_matches(|c: char| matches!(c, '.' | ':' | ';' | '?' | '!'));
        if word.contains('/') && !word.contains("://") && dir.join(word).exists() {
            areas.insert(area(word.trim_start_matches("./")));
        }
    }
    let db = elyra_index::default_db_path(&elyra_core::paths::data_dir().join("index"), dir);
    if db.exists() {
        let _one_at_a_time = crate::gateway::SYMBOL_LOOKUPS
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if let Ok(index) = elyra_index::SymbolIndex::open(dir, &db) {
            for word in crate::warm_start::words(message).iter().take(8) {
                for hit in index.definitions(word, true, None, 3).unwrap_or_default() {
                    areas.insert(area(&hit.file));
                }
            }
        }
    }
    areas
}

/// The records per project, kept for a few minutes: counting reads every
/// transcript of the project.
type Cache = HashMap<ProjectId, (Instant, std::sync::Arc<Vec<Record>>)>;
static CACHE: Mutex<Option<Cache>> = Mutex::new(None);
const CACHE_FOR: Duration = Duration::from_secs(300);

pub fn cached(project: ProjectId) -> Option<std::sync::Arc<Vec<Record>>> {
    let cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let (at, records) = cache.as_ref()?.get(&project)?;
    (at.elapsed() < CACHE_FOR).then(|| records.clone())
}

pub fn remember(project: ProjectId, records: std::sync::Arc<Vec<Record>>) {
    CACHE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(project, (Instant::now(), records));
}

/// The project's threads to count: (thread, provider, model).
pub fn threads(
    app: &crate::app_state::AppState,
    project: ProjectId,
) -> Vec<(ThreadId, ProviderKind, Option<String>)> {
    app.threads
        .iter()
        .chain(app.archived_threads().iter())
        .filter(|t| t.parent_id.is_none() && t.project_id == project)
        .map(|t| (t.id, t.provider, t.model.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{Record, area, recommend};
    use crate::leaderboard::Outcome;
    use elyra_core::ProviderKind;
    use std::collections::BTreeSet;

    fn record(provider: ProviderKind, area: &str, green: bool, cost: f64) -> Record {
        Record {
            provider,
            model: None,
            areas: BTreeSet::from([area.to_string()]),
            outcome: Outcome {
                first_try: Some(green),
                green: Some(green),
                fixes: 0,
                cost,
            },
        }
    }

    #[test]
    fn suggests_the_agent_that_did_best_there() {
        assert_eq!(area("app/Http/Controllers/CartController.php"), "app/Http");
        assert_eq!(area("src/lib.rs"), "src");
        assert_eq!(area("README.md"), "the project's root");
        let mut records = Vec::new();
        // Codex is good in the controllers, Claude Code in the views.
        for green in [true, true, true, false] {
            records.push(record(ProviderKind::Codex, "app/Http", green, 0.1));
            records.push(record(ProviderKind::Claude, "resources/views", true, 0.4));
        }
        for green in [true, false, false] {
            records.push(record(ProviderKind::Claude, "app/Http", green, 0.4));
            records.push(record(ProviderKind::Codex, "resources/views", green, 0.1));
        }
        let http = recommend(&records, &BTreeSet::from(["app/Http".to_string()])).unwrap();
        assert_eq!(http.provider, ProviderKind::Codex);
        assert!(
            http.reason.starts_with(
                "In app/Http, Codex has been green on the first try in 3 of 4 instructions"
            ),
            "{}",
            http.reason
        );
        let views = recommend(&records, &BTreeSet::from(["resources/views".to_string()])).unwrap();
        assert_eq!(views.provider, ProviderKind::Claude);
        // Elsewhere, or with too few to go on: nothing.
        assert!(recommend(&records, &BTreeSet::from(["database".to_string()])).is_none());
        assert!(recommend(&records[..4], &BTreeSet::from(["app/Http".to_string()])).is_none());
        assert!(recommend(&records, &BTreeSet::new()).is_none());
    }
}
