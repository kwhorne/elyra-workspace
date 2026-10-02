//! Read-only check of the GitHub integration against a real repository.
//!   cargo run -p elyra-git --example gh_smoke -- <repo-path>
use elyra_git::github::{self, ItemKind};

fn main() -> anyhow::Result<()> {
    let repo = std::path::PathBuf::from(std::env::args().nth(1).expect("repo path"));
    let slug = elyra_git::remote_url(&repo).and_then(|u| elyra_git::github_slug(&u));
    println!("slug: {slug:?}, gh available: {}", github::available());
    for (kind, open) in [
        (ItemKind::PullRequest, true),
        (ItemKind::PullRequest, false),
        (ItemKind::Issue, true),
    ] {
        let items = github::list_items(&repo, kind, open, 5)?;
        println!("{kind:?} open={open}: {} items", items.len());
        for item in items.iter().take(3) {
            println!(
                "  #{} {} [{}] by {} ({})",
                item.number, item.title, item.state, item.author, item.updated_at
            );
        }
        if kind == ItemKind::PullRequest
            && let Some(first) = items.first()
        {
            let pr = github::pr(&repo, first.number)?;
            println!(
                "  detail: {} → {}, checks {}, reviews {}, comments {}",
                pr.head,
                pr.base,
                pr.checks.len(),
                pr.reviews.len(),
                pr.comments.len()
            );
            let inline = github::review_comments(&repo, first.number)?;
            println!("  inline review comments: {}", inline.len());
        }
    }
    println!(
        "current branch PR: {:?}",
        github::pr_for_branch(&repo)?.map(|p| p.number)
    );
    Ok(())
}
