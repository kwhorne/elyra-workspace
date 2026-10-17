//! What fills a thread's context window: the conversation since the last
//! clear, estimated per message, tool call and tool result, next to what the
//! agent last reported. The rest of the reported context is the agent's own
//! setup (system prompt, tool definitions, instructions); older parts that no
//! longer fit were compacted away. Estimates are about four characters a token.

use elyra_core::{ItemContent, ItemId, TranscriptItem};
use std::collections::HashSet;

/// The notice a cleared context leaves in the transcript.
pub const CLEARED: &str = "Context cleared — the next message starts a new conversation.";

/// How many of the largest entries are listed.
const LARGEST: usize = 6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    You,
    Agent,
    ToolCall,
    ToolResult,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::You, Kind::Agent, Kind::ToolCall, Kind::ToolResult];

    pub fn label(self) -> &'static str {
        match self {
            Kind::You => "Your messages",
            Kind::Agent => "Agent replies",
            Kind::ToolCall => "Tool calls",
            Kind::ToolResult => "Tool results",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub item: ItemId,
    pub kind: Kind,
    pub label: String,
    pub tokens: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    /// What the agent last reported.
    pub used: Option<u64>,
    pub window: Option<u64>,
    /// Tokens and entries per kind, in [`Kind::ALL`] order.
    pub kinds: [(u64, usize); 4],
    /// The reported context the conversation doesn't account for: the
    /// agent's system prompt, tools and instructions.
    pub setup: Option<u64>,
    /// Older conversation that no longer fits the reported context.
    pub compacted: u64,
    /// The largest entries still in the context, largest first.
    pub largest: Vec<Entry>,
    /// What the last turn cost, to price the shares by.
    pub turn_cost: Option<f64>,
}

impl Report {
    /// The total to take shares of: what was reported, or the estimate.
    pub fn total(&self) -> u64 {
        self.used
            .unwrap_or_else(|| self.kinds.iter().map(|(tokens, _)| tokens).sum())
            .max(1)
    }

    pub fn share(&self, tokens: u64) -> f64 {
        tokens as f64 / self.total() as f64
    }

    /// About what carrying `tokens` cost in the last turn.
    pub fn cost(&self, tokens: u64) -> Option<f64> {
        self.turn_cost.map(|cost| cost * self.share(tokens))
    }
}

pub fn tokens(text: &str) -> u64 {
    (text.chars().count() as u64).div_ceil(4)
}

/// The report for a transcript, with the agent's last reported context use.
pub fn report(items: &[TranscriptItem], used: Option<u64>, window: Option<u64>) -> Report {
    let start = items
        .iter()
        .rposition(
            |item| matches!(&item.content, ItemContent::Notice { text, .. } if text == CLEARED),
        )
        .map_or(0, |index| index + 1);
    let items = &items[start..];
    // A subagent's own calls stay in its context, not the thread's.
    let subagent_calls: HashSet<&str> = items
        .iter()
        .filter_map(|item| match &item.content {
            ItemContent::ToolUse {
                tool_use_id,
                parent_tool_use_id: Some(_),
                ..
            } => Some(tool_use_id.as_str()),
            _ => None,
        })
        .collect();
    let mut entries: Vec<Entry> = items
        .iter()
        .filter_map(|item| {
            let (kind, label, text) = match &item.content {
                ItemContent::User { text, .. } => (Kind::You, first_line(text), text.clone()),
                ItemContent::Assistant {
                    text,
                    parent_tool_use_id: None,
                } => (Kind::Agent, first_line(text), text.clone()),
                ItemContent::ToolUse {
                    name,
                    input,
                    parent_tool_use_id: None,
                    ..
                } => (
                    Kind::ToolCall,
                    format!("{name} {}", crate::transcript::tool_summary(name, input)),
                    input.to_string(),
                ),
                ItemContent::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } if !subagent_calls.contains(tool_use_id.as_str()) => {
                    let call = items.iter().find_map(|i| match &i.content {
                        ItemContent::ToolUse {
                            tool_use_id: id,
                            name,
                            input,
                            ..
                        } if id == tool_use_id => Some(format!(
                            "{name} {}",
                            crate::transcript::tool_summary(name, input)
                        )),
                        _ => None,
                    });
                    (
                        Kind::ToolResult,
                        format!("Result of {}", call.unwrap_or_else(|| "a tool".into())),
                        content.clone(),
                    )
                }
                _ => return None,
            };
            Some(Entry {
                item: item.id,
                kind,
                label: label.trim().to_string(),
                tokens: tokens(&text),
            })
        })
        .collect();
    // Newest first, keep what fits the reported context: older parts were
    // compacted into a summary.
    let mut compacted = 0;
    if let Some(used) = used {
        let mut kept = 0;
        let mut cut = None;
        for (index, entry) in entries.iter().enumerate().rev() {
            if kept + entry.tokens > used {
                cut = Some(index);
                break;
            }
            kept += entry.tokens;
        }
        if let Some(cut) = cut {
            compacted = entries[..=cut].iter().map(|e| e.tokens).sum();
            entries.drain(..=cut);
        }
    }
    let mut kinds = [(0, 0); 4];
    for entry in &entries {
        let slot = &mut kinds[Kind::ALL.iter().position(|k| *k == entry.kind).unwrap_or(0)];
        slot.0 += entry.tokens;
        slot.1 += 1;
    }
    let conversation: u64 = kinds.iter().map(|(tokens, _)| tokens).sum();
    let setup = used.map(|used| used.saturating_sub(conversation));
    let turn_cost = items.iter().rev().find_map(|item| match &item.content {
        ItemContent::TurnSummary { cost_usd, .. } => *cost_usd,
        _ => None,
    });
    entries.sort_by_key(|entry| std::cmp::Reverse(entry.tokens));
    entries.truncate(LARGEST);
    Report {
        used,
        window,
        kinds,
        setup,
        compacted,
        largest: entries,
        turn_cost,
    }
}

/// A hint when one entry takes a large part of the context.
pub fn hint(report: &Report) -> Option<String> {
    let largest = report.largest.first()?;
    let share = report.share(largest.tokens);
    (share >= 0.15 && largest.kind == Kind::ToolResult).then(|| {
        format!(
            "One tool result takes {:.0} % of the context and is read again on every call the agent makes. Compacting summarises the conversation so far.",
            share * 100.
        )
    })
}

fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if line.chars().count() > 80 {
        format!("{}…", line.chars().take(79).collect::<String>())
    } else {
        line.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{CLEARED, Kind, hint, report};
    use chrono::Utc;
    use elyra_core::{ItemContent, TranscriptItem};
    use serde_json::json;

    fn item(content: ItemContent) -> TranscriptItem {
        TranscriptItem {
            id: elyra_core::new_id(),
            thread_id: elyra_core::new_id(),
            seq: 0,
            content,
            created_at: Utc::now(),
        }
    }

    fn user(text: &str) -> TranscriptItem {
        item(ItemContent::User {
            text: text.into(),
            checkpoint: None,
            attachments: Vec::new(),
        })
    }

    fn call(id: &str, command: &str, parent: Option<&str>) -> TranscriptItem {
        item(ItemContent::ToolUse {
            tool_use_id: id.into(),
            name: "Bash".into(),
            input: json!({ "command": command }),
            parent_tool_use_id: parent.map(str::to_string),
        })
    }

    fn result(id: &str, chars: usize) -> TranscriptItem {
        item(ItemContent::ToolResult {
            tool_use_id: id.into(),
            content: "x".repeat(chars),
            is_error: false,
        })
    }

    fn turn(cost: f64) -> TranscriptItem {
        item(ItemContent::TurnSummary {
            duration_ms: None,
            cost_usd: Some(cost),
            is_error: false,
            context_tokens: None,
            context_window: None,
            after: None,
        })
    }

    #[test]
    fn breaks_the_context_down() {
        let items = vec![
            user("Fix the failing test"),
            call("t1", "npm test", None),
            result("t1", 40_000),
            call("t2", "ls", Some("task")),
            result("t2", 4_000),
            item(ItemContent::Assistant {
                text: "Fixed it.".into(),
                parent_tool_use_id: None,
            }),
            turn(0.5),
        ];
        let report = report(&items, Some(50_000), Some(200_000));
        let results = report.kinds[3];
        assert_eq!(
            results,
            (10_000, 1),
            "the subagent's result isn't the thread's"
        );
        assert_eq!(report.kinds[0], (5, 1));
        let conversation: u64 = report.kinds.iter().map(|k| k.0).sum();
        assert_eq!(report.setup, Some(50_000 - conversation));
        assert_eq!(report.compacted, 0);
        let largest = &report.largest[0];
        assert_eq!(
            (largest.kind, largest.label.as_str(), largest.tokens),
            (Kind::ToolResult, "Result of Bash npm test", 10_000)
        );
        assert_eq!(report.share(10_000), 0.2);
        assert_eq!(report.cost(10_000), Some(0.1));
        assert!(
            hint(&report)
                .unwrap()
                .starts_with("One tool result takes 20 %")
        );
    }

    #[test]
    fn starts_after_a_clear_and_drops_what_was_compacted() {
        let items = vec![
            user("old"),
            item(ItemContent::Notice {
                text: CLEARED.into(),
                is_error: false,
            }),
            call("t1", "cat big.log", None),
            result("t1", 80_000),
            user("and now?"),
        ];
        let cleared = report(&items, None, None);
        assert_eq!(cleared.kinds[0].1, 1, "only the message after the clear");
        // The agent reports less than the conversation: the big result was
        // compacted away.
        let compacted = report(&items, Some(1_000), None);
        let call_tokens = super::tokens(&json!({ "command": "cat big.log" }).to_string());
        assert_eq!(
            compacted.compacted,
            20_000 + call_tokens,
            "the result and its call"
        );
        assert_eq!(compacted.kinds[3], (0, 0));
        assert!(hint(&compacted).is_none());
    }
}
