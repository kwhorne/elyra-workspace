//! The thrash guard: what happens when an agent goes in circles during a
//! turn (see `thrash_guard`).

use super::*;

impl ThreadSession {
    /// The agent is going in circles: say so, and ask what to do or start
    /// over (Settings → Stuck agents).
    pub(super) fn on_stuck(&mut self, stuck: crate::thrash_guard::Stuck, cx: &mut Context<Self>) {
        let mode = crate::preferences::Preferences::global(cx)
            .stuck_agents
            .clone();
        if mode == "off" || !self.running || self.stopped {
            return;
        }
        let start_over = mode == "start_over";
        self.append(
            ItemContent::Stuck {
                reason: stuck.reason.clone(),
                detail: stuck.detail,
                outcome: start_over.then_some(elyra_core::StuckOutcome::StartedOver),
            },
            cx,
        );
        if start_over {
            self.begin_start_over(stuck.reason, cx);
        } else {
            cx.emit(SessionEvent::NeedsAttention);
        }
    }

    /// The user's answer on a stuck card.
    pub fn resolve_stuck(
        &mut self,
        item: ItemId,
        outcome: elyra_core::StuckOutcome,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.items.iter().position(|i| i.id == item) else {
            return;
        };
        let ItemContent::Stuck {
            reason,
            outcome: current,
            ..
        } = &mut self.items[index].content
        else {
            return;
        };
        if current.is_some() {
            return;
        }
        *current = Some(outcome);
        let reason = reason.clone();
        self.persist_item(index, cx);
        match outcome {
            elyra_core::StuckOutcome::StartedOver => self.begin_start_over(reason, cx),
            elyra_core::StuckOutcome::Stopped => self.interrupt(cx),
            elyra_core::StuckOutcome::LetRun => cx.notify(),
        }
    }

    /// Stop the turn; it starts over once it has ended.
    pub(super) fn begin_start_over(&mut self, reason: String, cx: &mut Context<Self>) {
        if self.running {
            self.start_over = Some(reason);
            self.interrupt(cx);
        } else {
            self.start_over_now(reason, cx);
        }
    }

    /// Put the files back to the stuck turn's checkpoint and send the agent
    /// at it again, with the escalation model.
    pub(super) fn start_over_now(&mut self, reason: String, cx: &mut Context<Self>) {
        let checkpoint = self
            .items
            .iter()
            .rev()
            .find_map(|item| match &item.content {
                ItemContent::User { checkpoint, .. } => Some(checkpoint.clone()),
                _ => None,
            })
            .flatten();
        self.set_running(true, cx);
        self.activity = Some("Putting the files back…".into());
        cx.notify();
        let job = checkpoint.map(|sha| self.restore_job(sha, cx));
        cx.spawn(async move |this, cx| {
            let restored = match job {
                Some(job) => Some(job.await),
                None => None,
            };
            let _ = this.update(cx, |this, cx| {
                this.activity = None;
                let files = match restored {
                    Some(Ok(None | Some(Ok(())))) => {
                        "the files were put back to how they were before it"
                    }
                    Some(Ok(Some(Err(err)))) => {
                        this.notice(format!("Files restored, but not the database: {err:#}"), true, cx);
                        "the files were put back to how they were before it (the database wasn't)"
                    }
                    Some(Err(err)) => {
                        this.notice(format!("Could not restore files: {err:#}"), true, cx);
                        "the files couldn't be put back, so they are as it left them"
                    }
                    None => "the files are as it left them (there was no checkpoint)",
                };
                cx.emit(SessionEvent::FilesChanged);
                this.check_attempts = 0;
                this.escalate("The new attempt", cx);
                this.send(
                    Prompt::text(format!(
                        "{} ({reason}), so Elyra Workspace stopped it; {files}. \
                         Start again on the same request with a different approach, and don't repeat what failed.",
                        crate::checks::START_OVER_PROMPT_START
                    )),
                    cx,
                );
            });
        })
        .detach();
    }
}
