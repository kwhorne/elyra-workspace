//! The mutation gate: once the checks pass, small changes to the lines the
//! instruction added, to see whether the tests notice (see `mutation`).

use super::*;

impl ThreadSession {
    /// The checks passed in this turn: change the lines the instruction
    /// added one small way at a time, in a temporary checkout, and see
    /// whether they still pass (Settings → Test the tests after green checks).
    pub(crate) fn test_the_tests(&mut self, cx: &mut Context<Self>) {
        let changes = crate::preferences::Preferences::global(cx).mutation_changes as usize;
        if changes == 0 || self.skip_checks || self.stopped || self.mutation_cancel.is_some() {
            return;
        }
        let Some(command) = self.check_command() else {
            return;
        };
        // A check passed in this turn, quickly enough to run a few more times.
        let Some(turn) = self
            .items
            .iter()
            .rposition(|item| matches!(item.content, ItemContent::User { .. }))
        else {
            return;
        };
        let check = self.items[turn..]
            .iter()
            .rev()
            .find_map(|item| match &item.content {
                ItemContent::Check {
                    passed,
                    duration_ms,
                    ..
                } => Some((*passed, *duration_ms)),
                _ => None,
            });
        let Some((true, check_ms)) = check else {
            return;
        };
        let limit_secs = crate::preferences::Preferences::global(cx).mutation_max_check_secs as u64;
        if check_ms > limit_secs * 1000 {
            return;
        }
        // The folder before the instruction (its fixes and asked-for tests
        // belong to it); for Blue in a duel, before the duel's code.
        let blue = self.app.upgrade().is_some_and(|app| {
            crate::duel::of_thread(&app.read(cx).store, self.thread.id)
                .is_some_and(|duel| duel.blue == Some(self.thread.id))
        });
        let instruction = |item: &&TranscriptItem| match &item.content {
            ItemContent::User { text, .. } => {
                blue || crate::checks::fix_prompt_label(text).is_none()
            }
            _ => false,
        };
        let found = if blue {
            self.items.iter().find(instruction)
        } else {
            self.items.iter().rev().find(instruction)
        };
        let Some(before) = found.and_then(|item| match &item.content {
            ItemContent::User { checkpoint, .. } => checkpoint.clone(),
            _ => None,
        }) else {
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.mutation_cancel = Some(cancel.clone());
        let dir = self.working_dir();
        // What the tests caught before isn't tried again.
        let skip = crate::mutation::caught_before(&self.items);
        let job = cx.background_executor().spawn(async move {
            let root = elyra_git::repo_root(&dir)?;
            let after = elyra_git::checkpoint::snapshot(&root, "elyra: mutation gate")?;
            let candidates = crate::mutation::choose(&root, &before, &after, changes, &skip)?;
            anyhow::Ok((dir, root, after, candidates))
        });
        cx.spawn(async move |this, cx| {
            let chosen = job.await;
            let _ = this.update(cx, |this, cx| {
                let (dir, root, after, candidates) = match chosen {
                    Ok(chosen) if !chosen.3.is_empty() => chosen,
                    Ok(_) => {
                        this.mutation_cancel = None;
                        return;
                    }
                    Err(err) => {
                        log::warn!("mutation gate: {err:#}");
                        this.mutation_cancel = None;
                        return;
                    }
                };
                this.append(
                    ItemContent::MutationGate {
                        command: command.clone(),
                        mutants: candidates.iter().map(|c| c.result.clone()).collect(),
                        state: elyra_core::GateState::Running,
                        asked: false,
                    },
                    cx,
                );
                let Some(card) = this.items.last().map(|item| item.id) else {
                    return;
                };
                let limit = crate::mutation::limit_for(check_ms);
                let job = cx.background_executor().spawn(async move {
                    crate::mutation::run(&root, &dir, &after, &command, &candidates, limit, &cancel)
                });
                cx.spawn(async move |this, cx| {
                    let caught = job.await;
                    let _ = this.update(cx, |this, cx| this.mutation_done(card, caught, cx));
                })
                .detach();
            });
        })
        .detach();
    }

    pub(super) fn mutation_done(
        &mut self,
        card: ItemId,
        caught: anyhow::Result<Vec<bool>>,
        cx: &mut Context<Self>,
    ) {
        self.mutation_cancel = None;
        let Some(index) = self.items.iter().position(|item| item.id == card) else {
            return;
        };
        let ItemContent::MutationGate { mutants, state, .. } = &mut self.items[index].content
        else {
            return;
        };
        let mut done = false;
        match caught {
            Ok(caught) => {
                done = caught.len() == mutants.len();
                for (mutant, caught) in mutants.iter_mut().zip(caught) {
                    mutant.caught = Some(caught);
                }
                *state = if done {
                    elyra_core::GateState::Done
                } else {
                    elyra_core::GateState::Stopped
                };
            }
            Err(err) => {
                *state = elyra_core::GateState::Failed {
                    reason: format!("{err:#}"),
                };
            }
        }
        self.persist_item(index, cx);
        self.gates_done += 1;
        if done {
            cx.emit(SessionEvent::TestsTested);
        }
        cx.notify();
    }

    /// The last mutation gate that ran to the end: its card and results.
    pub fn tested_tests(&self) -> Option<(ItemId, Vec<elyra_core::MutantResult>)> {
        let item = self
            .items
            .iter()
            .rev()
            .find(|item| matches!(item.content, ItemContent::MutationGate { .. }))?;
        match &item.content {
            ItemContent::MutationGate {
                mutants,
                state: elyra_core::GateState::Done,
                ..
            } => Some((item.id, mutants.clone())),
            _ => None,
        }
    }

    /// What got through went elsewhere (to Red in a duel): no more asking here.
    pub fn handed_over(&mut self, card: ItemId, cx: &mut Context<Self>) {
        let Some(index) = self.items.iter().position(|item| item.id == card) else {
            return;
        };
        if let ItemContent::MutationGate { asked, .. } = &mut self.items[index].content {
            *asked = true;
            self.persist_item(index, cx);
            cx.notify();
        }
    }

    /// Stop the mutation gate.
    pub fn stop_mutation(&mut self, cx: &mut Context<Self>) {
        if let Some(cancel) = &self.mutation_cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        cx.notify();
    }

    /// Ask the agent for tests that catch the changes that got through.
    pub fn ask_for_tests(&mut self, card: ItemId, cx: &mut Context<Self>) {
        let Some(index) = self.items.iter().position(|item| item.id == card) else {
            return;
        };
        let ItemContent::MutationGate {
            command,
            mutants,
            asked,
            ..
        } = &mut self.items[index].content
        else {
            return;
        };
        let Some(prompt) = crate::mutation::ask_prompt(command, mutants) else {
            return;
        };
        *asked = true;
        self.persist_item(index, cx);
        self.submit(Prompt::text(prompt), cx);
    }
}
