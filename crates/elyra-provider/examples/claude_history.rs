//! List recent Claude Code sessions that can be imported.
//!   cargo run -p elyra-provider --example claude_history

fn main() {
    let root = elyra_provider::claude_history::projects_dir().expect("home directory");
    let started = std::time::Instant::now();
    let sessions = elyra_provider::claude_history::list_sessions(&root, 15);
    for s in &sessions {
        println!(
            "{:<38} {:>4} msgs  {:<40}  {}",
            s.session_id,
            s.messages,
            s.title.chars().take(40).collect::<String>(),
            s.cwd
                .as_ref()
                .map(|c| c.display().to_string())
                .unwrap_or_default()
        );
    }
    if let Some(first) = sessions.first() {
        let session = elyra_provider::claude_history::load_session(&first.path).unwrap();
        println!("first session: {} items", session.items.len());
    }
    println!("listed in {:?}", started.elapsed());
}
