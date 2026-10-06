//! Export a thread as a ZIP: a readable Markdown transcript, the raw
//! transcript JSON and the thread's metadata.

use crate::app_state::AppState;
use elyra_core::{ItemContent, ThreadId};
use std::io::Write as _;
use std::path::Path;

pub fn markdown(title: &str, items: &[elyra_core::TranscriptItem]) -> String {
    let mut out = format!("# {title}\n");
    for item in items {
        let time = item
            .created_at
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M");
        match &item.content {
            ItemContent::User { text, .. } => {
                out.push_str(&format!("\n## You · {time}\n\n{text}\n"))
            }
            ItemContent::Assistant {
                text,
                parent_tool_use_id: None,
            } => out.push_str(&format!("\n## Agent · {time}\n\n{text}\n")),
            ItemContent::ToolUse {
                name,
                input,
                parent_tool_use_id: None,
                ..
            } => out.push_str(&format!(
                "\n> **{name}** {}\n",
                crate::transcript::tool_summary(name, input)
            )),
            ItemContent::TurnSummary {
                duration_ms,
                cost_usd,
                ..
            } => out.push_str(&format!(
                "\n_Turn finished{}{}_\n",
                duration_ms
                    .map(|ms| format!(" in {:.1} s", ms as f64 / 1000.))
                    .unwrap_or_default(),
                cost_usd.map(|c| format!(" · ${c:.3}")).unwrap_or_default()
            )),
            ItemContent::Notice { text, .. } => out.push_str(&format!("\n> {text}\n")),
            ItemContent::Check {
                command, passed, ..
            } => out.push_str(&format!(
                "\n> Checks {}: `{command}`\n",
                if *passed { "passed" } else { "failed" }
            )),
            _ => {}
        }
    }
    out
}

pub fn export_thread(app: &AppState, id: ThreadId, target: &Path) -> anyhow::Result<()> {
    let thread = app
        .thread(id)
        .cloned()
        .or_else(|| app.archived_threads().into_iter().find(|t| t.id == id))
        .ok_or_else(|| anyhow::anyhow!("thread not found"))?;
    let items = app.store.transcript(id)?;
    let file = std::fs::File::create(target)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    zip.start_file("transcript.md", options)?;
    zip.write_all(markdown(&thread.title, &items).as_bytes())?;
    zip.start_file("transcript.json", options)?;
    zip.write_all(serde_json::to_string_pretty(&items)?.as_bytes())?;
    zip.start_file("thread.json", options)?;
    zip.write_all(serde_json::to_string_pretty(&thread)?.as_bytes())?;
    zip.finish()?;
    Ok(())
}

/// A file name for the export from the thread title.
pub fn file_name(title: &str) -> String {
    let slug: String = title
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!(
        "{}.zip",
        if slug.is_empty() {
            "thread".to_string()
        } else {
            slug.chars().take(60).collect()
        }
    )
}

#[cfg(test)]
mod tests {
    use super::{file_name, markdown};
    use elyra_core::{ItemContent, TranscriptItem};

    #[test]
    fn renders_markdown_and_file_names() {
        let item = |content| TranscriptItem {
            id: elyra_core::new_id(),
            thread_id: elyra_core::new_id(),
            seq: 1,
            content,
            created_at: chrono::Utc::now(),
        };
        let md = markdown(
            "Fix it",
            &[
                item(ItemContent::User {
                    text: "Please fix".into(),
                    checkpoint: None,
                }),
                item(ItemContent::Assistant {
                    text: "Done".into(),
                    parent_tool_use_id: None,
                }),
            ],
        );
        assert!(md.starts_with("# Fix it\n"));
        assert!(md.contains("Please fix") && md.contains("Done"));
        assert_eq!(file_name("Fix: the *login* bug!"), "Fix-the-login-bug.zip");
    }
}
