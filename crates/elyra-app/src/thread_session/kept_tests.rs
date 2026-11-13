//! Keeping an error a turn fixed as a regression test, and checking that
//! the test catches it.

use super::*;

impl ThreadSession {
    /// The turn worked on errors: offer to keep them as a regression test.
    pub(super) fn offer_keep_as_test(&mut self, cx: &mut Context<Self>) {
        let errors = std::mem::take(&mut self.errors_in_turn);
        if errors.is_empty() {
            return;
        }
        let Some(before) = self
            .items
            .iter()
            .rev()
            .find_map(|item| match &item.content {
                ItemContent::User { checkpoint, .. } => checkpoint.clone(),
                _ => None,
            })
        else {
            return;
        };
        let (titles, details): (Vec<String>, Vec<String>) = errors.into_iter().unzip();
        self.append(
            ItemContent::ErrorFixed {
                errors: titles,
                detail: details.join("\n\n---\n\n").chars().take(8_000).collect(),
                before,
                test: None,
            },
            cx,
        );
    }

    pub(super) fn set_kept_test(
        &mut self,
        card: ItemId,
        state: elyra_core::KeptTest,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.items.iter().position(|i| i.id == card) else {
            return;
        };
        if let ItemContent::ErrorFixed { test, .. } = &mut self.items[index].content {
            *test = Some(state);
            self.persist_item(index, cx);
            cx.notify();
        }
    }

    /// Have the agent write the test for the errors on `card` (again, after a
    /// test that didn't guard the fix).
    pub fn keep_as_test(&mut self, card: ItemId, cx: &mut Context<Self>) {
        let Some(ItemContent::ErrorFixed {
            errors,
            detail,
            test,
            ..
        }) = self
            .items
            .iter()
            .find(|i| i.id == card)
            .map(|i| i.content.clone())
        else {
            return;
        };
        let prompt = test
            .as_ref()
            .and_then(crate::keep_test::retry_prompt)
            .unwrap_or_else(|| crate::keep_test::prompt(&errors, &detail));
        self.kept_test = Some(card);
        self.set_kept_test(card, elyra_core::KeptTest::Writing, cx);
        self.submit(Prompt::text(prompt), cx);
    }

    /// The turn writing a kept test ended: run it with and without the fix.
    pub(super) fn check_kept_test(&mut self, cx: &mut Context<Self>) {
        let Some(card) = self.kept_test else {
            return;
        };
        let asked = self
            .items
            .iter()
            .rev()
            .find_map(|item| match &item.content {
                ItemContent::User {
                    text, checkpoint, ..
                } => Some((text.starts_with(crate::keep_test::ASK), checkpoint.clone())),
                _ => None,
            });
        let Some((true, Some(test_before))) = asked else {
            return;
        };
        self.kept_test = None;
        let Some(ItemContent::ErrorFixed { before, .. }) = self
            .items
            .iter()
            .find(|i| i.id == card)
            .map(|i| i.content.clone())
        else {
            return;
        };
        self.set_kept_test(card, elyra_core::KeptTest::Checking, cx);
        let dir = self.working_dir();
        let command = self.check_command();
        let cancel = Arc::new(AtomicBool::new(false));
        let job = cx.background_executor().spawn(async move {
            let root = match elyra_git::repo_root(&dir) {
                Ok(root) => root,
                Err(err) => {
                    return elyra_core::KeptTest::NotVerified {
                        reason: format!("{err:#}"),
                    };
                }
            };
            let after = match elyra_git::checkpoint::snapshot(&root, "elyra: kept test") {
                Ok(after) => after,
                Err(err) => {
                    return elyra_core::KeptTest::NotVerified {
                        reason: format!("{err:#}"),
                    };
                }
            };
            let folder = dir
                .strip_prefix(&root)
                .unwrap_or(std::path::Path::new(""))
                .to_path_buf();
            crate::keep_test::verify(
                &root,
                &folder,
                &before,
                &test_before,
                &after,
                command.as_deref(),
                &cancel,
            )
        });
        cx.spawn(async move |this, cx| {
            let verdict = job.await;
            let _ = this.update(cx, |this, cx| this.set_kept_test(card, verdict, cx));
        })
        .detach();
    }
}
