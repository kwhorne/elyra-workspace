//! Done means green: the project's checks (and the invariants, and the
//! journeys) after a turn that changed files, the automatic fixes when they
//! fail, and the stronger model a second fix gets.

use super::*;

impl ThreadSession {
    /// The project's check command, if it has one (and the thread runs it).
    pub fn check_command(&self) -> Option<String> {
        if self.skip_checks {
            return None;
        }
        self.project
            .check_command
            .as_deref()
            .map(str::trim)
            .filter(|command| !command.is_empty())
            .map(str::to_string)
    }

    /// Run the checks if the turn changed files; the turn stays running
    /// until they are done.
    pub(super) fn run_checks(
        &mut self,
        command: Option<String>,
        spent_before: f64,
        cx: &mut Context<Self>,
    ) {
        let dir = self.working_dir();
        // The snapshot taken before this turn's message reached the agent.
        let checkpoint = self
            .items
            .iter()
            .rev()
            .find_map(|item| match &item.content {
                ItemContent::User { checkpoint, .. } => Some(checkpoint.clone()),
                _ => None,
            });
        let cancel = Arc::new(AtomicBool::new(false));
        self.check_cancel = Some(cancel.clone());
        if let Some(command) = &command {
            self.activity = Some(format!("Running checks: {command}"));
        }
        cx.notify();
        let run_command = command.clone();
        let invariants = !self.skip_checks;
        // Invariants look at all the instruction changed: a fix that didn't
        // touch the file that broke one hasn't mended it.
        let instruction = self
            .items
            .iter()
            .rev()
            .find_map(|item| match &item.content {
                ItemContent::User {
                    text, checkpoint, ..
                } if crate::checks::fix_prompt_label(text).is_none() => Some(checkpoint.clone()),
                _ => None,
            })
            .flatten();
        // None: the turn changed no files; Some(None): no check command.
        let job = cx.background_executor().spawn(async move {
            let checkpoint = checkpoint.flatten();
            if !crate::checks::changed_since(&dir, checkpoint.as_deref()) {
                return (None, None);
            }
            // The project's invariants first: quick, and when they break the
            // slower checks needn't run.
            let held = invariants
                .then(|| {
                    let root = elyra_git::repo_root(&dir).ok()?;
                    let (invariants, problems) = crate::invariants::load(&root);
                    for problem in problems {
                        log::warn!("invariant: {problem}");
                    }
                    if invariants.is_empty() {
                        return None;
                    }
                    let base = instruction.as_deref().or(checkpoint.as_deref())?;
                    let changed = crate::invariants::changes(&root, base)
                        .map_err(|err| log::warn!("invariants: {err:#}"))
                        .ok()?;
                    let (checked, broken) = crate::invariants::check(&root, &invariants, &changed);
                    (!checked.is_empty()).then_some((checked, broken))
                })
                .flatten();
            if held.as_ref().is_some_and(|(_, broken)| !broken.is_empty()) {
                return (held, Some(None));
            }
            let outcome = run_command.map(|command| crate::checks::run(&dir, &command, &cancel));
            (held, Some(outcome))
        });
        cx.spawn(async move |this, cx| {
            let (held, outcome) = job.await;
            let _ = this.update(cx, |this, cx| {
                if let Some((checked, broken)) = held {
                    this.append(
                        ItemContent::Invariants {
                            checked,
                            broken: broken.clone(),
                        },
                        cx,
                    );
                    if !broken.is_empty() && !this.stopped {
                        this.check_cancel = None;
                        this.fix_or_hand_over(
                            "The invariants",
                            |attempt, attempts| {
                                crate::invariants::fix_prompt(&broken, attempt, attempts)
                            },
                            spent_before,
                            cx,
                        );
                        return;
                    }
                }
                this.checks_done(command, outcome, spent_before, cx)
            });
        })
        .detach();
    }

    pub(super) fn checks_done(
        &mut self,
        command: Option<String>,
        outcome: Option<Option<crate::checks::Outcome>>,
        spent_before: f64,
        cx: &mut Context<Self>,
    ) {
        self.check_cancel = None;
        // The turn changed no files.
        let Some(outcome) = outcome else {
            self.finish_turn(false, spent_before, cx);
            return;
        };
        // No check command: on to the journeys.
        let (Some(command), Some(outcome)) = (command, outcome) else {
            self.replay_journeys(spent_before, cx);
            return;
        };
        let passed = outcome.passed;
        self.append(
            ItemContent::Check {
                command: command.clone(),
                passed,
                exit_code: outcome.exit_code,
                duration_ms: outcome.duration_ms,
                output: outcome.output.clone(),
            },
            cx,
        );
        if self.stopped {
            self.finish_turn(false, spent_before, cx);
            return;
        }
        if passed {
            self.replay_journeys(spent_before, cx);
            return;
        }
        self.fix_or_hand_over(
            "The checks",
            |attempt, attempts| crate::checks::fix_prompt(&command, &outcome, attempt, attempts),
            spent_before,
            cx,
        );
    }

    /// Something failed after the turn: send it back to the agent while
    /// attempts are left, else hand the thread to the user.
    pub(super) fn fix_or_hand_over(
        &mut self,
        what: &str,
        prompt: impl FnOnce(u32, u32) -> String,
        spent_before: f64,
        cx: &mut Context<Self>,
    ) {
        let attempts = crate::preferences::Preferences::global(cx).check_fix_attempts;
        let can_fix = self.check_attempts < attempts
            && self.queued.is_empty()
            && self.budget_reached().is_none();
        if can_fix {
            self.check_attempts += 1;
            // A fix already failed: the next one gets a stronger model.
            if self.check_attempts >= 2 {
                self.escalate("The first fix didn't do it; the next one", cx);
            }
            let prompt = prompt(self.check_attempts, attempts);
            self.activity = None;
            self.send(Prompt::text(prompt), cx);
            return;
        }
        if attempts > 0 && self.check_attempts >= attempts {
            self.notice(
                format!("{what} still fail after {attempts} automatic fixes; over to you."),
                true,
                cx,
            );
        }
        self.finish_turn(true, spent_before, cx);
    }

    /// Switch to the escalation model and the highest effort for the next
    /// fix, remembering what to go back to.
    /// `lead` says what gets it ("The next attempt").
    pub(super) fn escalate(&mut self, lead: &str, cx: &mut Context<Self>) {
        if self.escalated_from.is_some() {
            return;
        }
        let configured = crate::preferences::Preferences::global(cx)
            .provider(self.thread.provider)
            .escalation_model
            .trim()
            .to_string();
        let model = Some(configured)
            .filter(|m| !m.is_empty())
            .or_else(|| self.thread.model.clone());
        let effort = self
            .capabilities()
            .efforts
            .last()
            .map(|(id, _)| id.to_string())
            .or_else(|| self.thread.effort.clone());
        if model == self.thread.model && effort == self.thread.effort {
            return;
        }
        self.escalated_from = Some((self.thread.model.clone(), self.thread.effort.clone()));
        let mut parts = Vec::new();
        if model != self.thread.model {
            parts.push(format!("model {}", model.as_deref().unwrap_or("default")));
        }
        if effort != self.thread.effort {
            parts.push(format!("effort {}", effort.as_deref().unwrap_or("default")));
        }
        self.notice(
            format!("{lead} gets {} (back afterwards).", parts.join(" and ")),
            false,
            cx,
        );
        self.apply_model_effort(model, effort, cx);
    }

    /// Change the thread's model and effort without making them the
    /// provider's defaults.
    pub(super) fn apply_model_effort(
        &mut self,
        model: Option<String>,
        effort: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.thread.model != model {
            self.thread.model = model;
            if let Some(provider) = &self.provider
                && let Err(err) = provider.set_model(self.thread.model.as_deref())
            {
                log::warn!("set_model failed: {err:#}");
                self.needs_restart = true;
            }
            // Going back to the provider's default needs a fresh start.
            if self.thread.model.is_none() {
                self.needs_restart = true;
            }
        }
        if self.thread.effort != effort {
            self.thread.effort = effort;
            if let Some(provider) = &self.provider {
                match provider.set_effort(self.thread.effort.as_deref()) {
                    Ok(true) => {}
                    Ok(false) => self.needs_restart = true,
                    Err(err) => log::warn!("set_effort failed: {err:#}"),
                }
            }
        }
        self.save_thread(cx);
        cx.notify();
    }

    /// Run the checks now, outside a turn (to try the command): the result is
    /// shown but not sent to the agent.
    pub fn run_checks_now(&mut self, cx: &mut Context<Self>) {
        if self.running || self.check_cancel.is_some() {
            return;
        }
        let Some(command) = self.check_command() else {
            return;
        };
        let dir = self.working_dir();
        let cancel = Arc::new(AtomicBool::new(false));
        self.check_cancel = Some(cancel.clone());
        self.activity = Some(format!("Running checks: {command}"));
        cx.notify();
        let run_command = command.clone();
        let mine = cancel.clone();
        let job = cx
            .background_executor()
            .spawn(async move { crate::checks::run(&dir, &run_command, &cancel) });
        cx.spawn(async move |this, cx| {
            let outcome = job.await;
            let _ = this.update(cx, |this, cx| {
                // A turn may have started meanwhile, with checks of its own.
                if this
                    .check_cancel
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, &mine))
                {
                    this.check_cancel = None;
                    this.activity = None;
                }
                this.append(
                    ItemContent::Check {
                        command,
                        passed: outcome.passed,
                        exit_code: outcome.exit_code,
                        duration_ms: outcome.duration_ms,
                        output: outcome.output,
                    },
                    cx,
                );
            });
        })
        .detach();
    }
}
