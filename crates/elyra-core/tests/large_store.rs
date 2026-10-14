//! How the store holds up with a lot of history: 200 threads, 20 of them long
//! (2,000 entries each, tool output of a few KB). Ignored by default; run with
//!
//!     cargo test -p elyra-core --release --test large_store -- --ignored --nocapture

use elyra_core::{
    Environment, ItemContent, PermissionMode, ProviderKind, SearchFilter, Store, line_hash,
};
use std::time::Instant;

fn timed<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let value = f();
    println!(
        "{label:<48} {:>9.2} ms",
        start.elapsed().as_secs_f64() * 1000.0
    );
    value
}

fn items(turns: usize, seed: usize) -> Vec<(ItemContent, Option<chrono::DateTime<chrono::Utc>>)> {
    let mut items = Vec::new();
    for turn in 0..turns {
        let n = seed * 1000 + turn;
        items.push((
            ItemContent::User {
                text: format!("Turn {n}: fix the cart total when a discount over 100 % applies"),
                checkpoint: Some(format!("{n:040x}")),
            },
            None,
        ));
        for call in 0..3 {
            let id = format!("t{n}-{call}");
            items.push((
                ItemContent::ToolUse {
                    tool_use_id: id.clone(),
                    name: "Bash".into(),
                    input: serde_json::json!({ "command": format!("php artisan test --filter Cart{n}") }),
                    parent_tool_use_id: None,
                },
                None,
            ));
            items.push((
                ItemContent::ToolResult {
                    tool_use_id: id,
                    content: format!("PASS Tests\\Feature\\CartTest{n} line {call}\n").repeat(60),
                    is_error: false,
                },
                None,
            ));
        }
        items.push((
            ItemContent::Assistant {
                text: format!("## Fixed turn {n}\n\nThe total now rounds after the discount. ")
                    .repeat(8),
                parent_tool_use_id: None,
            },
            None,
        ));
        items.push((
            ItemContent::TurnSummary {
                duration_ms: Some(42_000),
                cost_usd: Some(0.12),
                is_error: false,
                context_tokens: Some(50_000),
                context_window: Some(200_000),
            },
            None,
        ));
    }
    items
}

#[test]
#[ignore]
fn holds_up_with_a_lot_of_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("elyra.sqlite");
    let store = timed("open + migrations (empty)", || Store::open(&path).unwrap());
    let project = store.add_project(dir.path()).unwrap();
    let mut long = Vec::new();
    timed("write 200 threads (20 × ~2,000 items)", || {
        for index in 0..200 {
            let thread = store
                .create_thread(
                    project.id,
                    ProviderKind::Claude,
                    None,
                    PermissionMode::Ask,
                    Environment::Local,
                )
                .unwrap();
            let turns = if index < 20 { 250 } else { 10 };
            store.append_items(thread.id, items(turns, index)).unwrap();
            if index < 20 {
                long.push(thread.id);
            }
        }
    });
    let size = std::fs::metadata(&path).unwrap().len();
    println!("{:<48} {:>9.1} MB", "database size", size as f64 / 1e6);
    drop(store);
    let store = timed("reopen", || Store::open(&path).unwrap());
    timed("threads (list all)", || store.threads(false).unwrap());
    timed("archived_threads", || store.archived_threads().unwrap());
    let transcript = timed("transcript of a 2,000-item thread", || {
        store.transcript(long[0]).unwrap()
    });
    println!("{:<48} {:>9}", "  items", transcript.len());
    timed("append_item to a long thread", || {
        store
            .append_item(
                long[0],
                ItemContent::Assistant {
                    text: "One more reply".into(),
                    parent_tool_use_id: None,
                },
            )
            .unwrap()
    });
    let mut last = transcript.last().unwrap().clone();
    timed("update_item in a long thread", || {
        last.content = ItemContent::Assistant {
            text: "Edited".into(),
            parent_tool_use_id: None,
        };
        store.update_item(&last).unwrap()
    });
    timed("turn_stats", || store.turn_stats().unwrap());
    let all = SearchFilter::default();
    let hits = timed("search_transcripts \"discount rounds\"", || {
        store
            .search_transcripts("discount rounds", &all, 200)
            .unwrap()
    });
    println!("{:<48} {:>9}", "  hits", hits.len());
    timed("search_transcripts \"CartTest1234\" (rare)", || {
        store.search_transcripts("CartTest1234", &all, 200).unwrap()
    });
    timed("search_messages (palette) \"cart\"", || {
        store.search_messages("cart", 8).unwrap()
    });
    let lines: Vec<(String, i64)> = (0..2_000)
        .map(|i| {
            (
                format!("src/file{}.rs", i % 50),
                line_hash(&format!("let value_{i} = compute({i});")).unwrap(),
            )
        })
        .collect();
    timed("record_line_origins (2,000 lines)", || {
        store
            .record_line_origins(
                project.id,
                long[0],
                transcript[0].id,
                chrono::Utc::now(),
                &lines,
            )
            .unwrap()
    });
    timed("unreviewed_lines (one file)", || {
        store.unreviewed_lines(project.id, "src/file7.rs").unwrap()
    });
    timed("setting lookup", || store.setting("preferences").unwrap());
    let reader = timed("open_read_only", || Store::open_read_only(&path).unwrap());
    let read = timed("search_transcripts on the read-only connection", || {
        reader
            .search_transcripts("discount rounds", &all, 200)
            .unwrap()
    });
    assert_eq!(read.len(), hits.len());
}
