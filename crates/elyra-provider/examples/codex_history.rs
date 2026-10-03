//! List recent Codex threads that can be imported.
//!   cargo run -p elyra-provider --example codex_history [path/to/codex]

fn main() -> anyhow::Result<()> {
    let codex = std::env::args()
        .nth(1)
        .map(std::path::PathBuf::from)
        .or_else(elyra_provider::codex::find_executable)
        .expect("codex not found");
    for s in elyra_provider::codex_history::list_sessions(&codex, 15)? {
        println!(
            "{}  {:<40} {}",
            s.session_id,
            s.title,
            s.cwd.map(|c| c.display().to_string()).unwrap_or_default()
        );
    }
    Ok(())
}
