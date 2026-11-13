//! Replaying the project's browser journeys after a turn, and accepting a
//! changed layout or query count.

use super::*;

impl ThreadSession {
    /// Whether the project replays its journeys after each turn (and has any).
    pub(super) fn journeys_after_turn(&self, cx: &App) -> bool {
        let Some(app) = self.app.upgrade() else {
            return false;
        };
        crate::journeys::replay_after_turn(&app.read(cx).store, self.project.id)
            && !crate::journeys::load_all(&self.working_dir()).is_empty()
    }

    /// After the checks passed: replay the project's journeys (when it does
    /// that after each turn), then end the turn.
    pub(super) fn replay_journeys(&mut self, spent_before: f64, cx: &mut Context<Self>) {
        if !self.journeys_after_turn(cx) || !cx.has_global::<crate::browser_tools::BrowserHub>() {
            self.finish_turn(false, spent_before, cx);
            return;
        }
        let journeys: Vec<crate::journeys::Journey> =
            crate::journeys::load_all(&self.working_dir())
                .into_iter()
                .filter_map(|(_, journey)| journey.ok())
                .collect();
        let thread = self.thread.id;
        self.activity = Some("Replaying journeys".into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            for journey in journeys {
                let stopped = this.update(cx, |this, _| this.stopped).unwrap_or(true);
                if stopped {
                    break;
                }
                let mut outcome = crate::browser_tools::replay(thread, &journey, cx).await;
                let _ = this.update(cx, |this, _| {
                    crate::journeys::guard(&this.working_dir(), &journey, &mut outcome)
                });
                let failed = this
                    .update(cx, |this, cx| {
                        this.append_journey(&journey, &outcome, cx);
                        if outcome.passed {
                            return false;
                        }
                        let name = journey.name.clone();
                        this.fix_or_hand_over(
                            "The journeys",
                            |attempt, attempts| {
                                crate::journeys::fix_prompt(&name, &outcome, attempt, attempts)
                            },
                            spent_before,
                            cx,
                        );
                        true
                    })
                    .unwrap_or(true);
                if failed {
                    return;
                }
            }
            let _ = this.update(cx, |this, cx| this.finish_turn(false, spent_before, cx));
        })
        .detach();
    }

    pub(super) fn append_journey(
        &mut self,
        journey: &crate::journeys::Journey,
        outcome: &crate::journeys::Outcome,
        cx: &mut Context<Self>,
    ) {
        self.append(
            ItemContent::Journey {
                name: journey.name.clone(),
                passed: outcome.passed,
                steps_run: outcome.steps_run,
                total_steps: journey.steps.len(),
                failure: outcome.failure.clone(),
                duration_ms: outcome.duration_ms,
                layout: outcome.layout_new.clone(),
                layout_saved: outcome.layout_saved,
                layout_accepted: false,
                queries: outcome.query_jumps.clone(),
            },
            cx,
        );
    }

    /// Accept a journey's new layout problems as how its pages look now.
    pub fn accept_layout(&mut self, item: ItemId, cx: &mut Context<Self>) {
        let Some(index) = self.items.iter().position(|i| i.id == item) else {
            return;
        };
        let dir = self.working_dir();
        if let ItemContent::Journey {
            name,
            layout,
            layout_accepted,
            queries,
            ..
        } = &mut self.items[index].content
        {
            crate::journeys::accept_layout(&dir, name, layout, queries);
            *layout_accepted = true;
            self.persist_item(index, cx);
            cx.notify();
        }
    }

    /// Replay journeys now (from the Context tab): shown, not sent to the agent.
    pub fn run_journeys_now(&mut self, name: Option<String>, cx: &mut Context<Self>) {
        if self.running || self.activity.is_some() {
            return;
        }
        let journeys: Vec<crate::journeys::Journey> =
            crate::journeys::load_all(&self.working_dir())
                .into_iter()
                .filter_map(|(_, journey)| journey.ok())
                .filter(|j| name.as_deref().is_none_or(|name| j.name == name))
                .collect();
        let thread = self.thread.id;
        self.activity = Some("Replaying journeys".into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            for journey in journeys {
                let mut outcome = crate::browser_tools::replay(thread, &journey, cx).await;
                let _ = this.update(cx, |this, cx| {
                    crate::journeys::guard(&this.working_dir(), &journey, &mut outcome);
                    this.append_journey(&journey, &outcome, cx)
                });
            }
            let _ = this.update(cx, |this, cx| {
                this.activity = None;
                cx.notify();
            });
        })
        .detach();
    }
}
