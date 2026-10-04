//! Live smoke test of a provider adapter through the AgentSession trait.
//!   cargo run -p elyra-provider --example smoke -- <provider> <cwd> "<prompt>"
//! For `acp`, set `ACP_COMMAND` to the agent command line.
//! Prints every provider event; auto-allows permission requests and answers
//! questions with the first option.

use elyra_core::{PermissionMode, ProviderKind, QuestionAnswer};
use elyra_provider::{PermissionResponse, Prompt, ProviderEvent, SessionConfig};
use std::path::PathBuf;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let kind = ProviderKind::parse(&args.next().unwrap_or_default())
        .expect("provider: claude|elyra|gemini|…|acp");
    let mut command = std::env::var("ACP_COMMAND")
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>()
        .into_iter();
    let cwd = PathBuf::from(args.next().expect("cwd"));
    let prompt = args.next().expect("prompt");
    let (session, events) = elyra_provider::start_session(
        kind,
        SessionConfig {
            cwd,
            model: None,
            effort: None,
            permission_mode: PermissionMode::Ask,
            resume_session_id: None,
            executable: command.next().map(PathBuf::from),
            args: command.collect(),
            env: Vec::new(),
            fork: false,
            append_system_prompt: None,
            // ELYRA_MCP_URL + ELYRA_MCP_TOKEN: give the agent an MCP server.
            mcp_servers: match (
                std::env::var("ELYRA_MCP_URL"),
                std::env::var("ELYRA_MCP_TOKEN"),
            ) {
                (Ok(url), Ok(token)) => vec![elyra_provider::McpServer {
                    stdio: None,
                    name: "elyra".into(),
                    url,
                    token,
                    bridge: None,
                }],
                _ => Vec::new(),
            },
        },
    )?;
    session.send(&Prompt::text(prompt))?;
    let mut deltas = 0;
    while let Ok(event) = events.recv_blocking() {
        match &event {
            ProviderEvent::TextDelta(_) | ProviderEvent::ThinkingDelta(_) => {
                deltas += 1;
                continue;
            }
            ProviderEvent::Commands(c) => println!("Commands({})", c.len()),
            ProviderEvent::Models(m) => println!(
                "Models({}): {:?}",
                m.len(),
                m.iter().take(3).map(|m| &m.id).collect::<Vec<_>>()
            ),
            ProviderEvent::PermissionRequest(r) => {
                println!("PermissionRequest {} {}", r.tool_name, r.input);
                session.respond_permission(&r.request_id, PermissionResponse::Allow)?;
            }
            ProviderEvent::Question(q) => {
                println!("Question {:?}", q.questions);
                let first = q
                    .questions
                    .iter()
                    .map(|q| {
                        q.options
                            .first()
                            .map(|o| o.label.clone())
                            .unwrap_or_default()
                    })
                    .collect();
                session
                    .answer_question(&q.request_id, QuestionAnswer::Answers { answers: first })?;
            }
            other => println!("{other:?}"),
        }
        if let ProviderEvent::TurnCompleted { .. } = event {
            println!("({deltas} streaming deltas)");
            if kind == ProviderKind::Elyra {
                // Elyra reports context usage on request.
                session.refresh_usage()?;
                if let Ok(ProviderEvent::Usage(usage)) = events.recv_blocking() {
                    println!("final usage: {usage:?}");
                }
            }
            break;
        }
        if let ProviderEvent::Exited { .. } = event {
            break;
        }
    }
    session.shutdown();
    Ok(())
}
