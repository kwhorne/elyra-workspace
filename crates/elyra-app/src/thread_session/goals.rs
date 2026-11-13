//! Thread goals (turns until the agent reports it achieved) and the
//! thread's budget.

use super::*;

impl ThreadSession {
    /// Set (or clear) the thread's goal and start working on it.
    pub fn set_goal(&mut self, goal: Option<String>, cx: &mut Context<Self>) {
        let goal = goal.map(|g| g.trim().to_string()).filter(|g| !g.is_empty());
        self.thread.goal_status = goal.as_ref().map(|_| GoalStatus::Active);
        self.thread.goal = goal;
        self.thread.goal_runs = 0;
        if self.thread.goal.is_some() && self.refuse_at_budget(cx) {
            self.thread.goal_status = Some(GoalStatus::Paused);
        }
        self.save_thread(cx);
        if self.thread.goal_status == Some(GoalStatus::Active)
            && !self.running
            && self.preparing.is_none()
        {
            let prompt = self.goal_prompt(true);
            self.submit(Prompt::text(prompt), cx);
        }
        cx.notify();
    }

    pub fn pause_goal(&mut self, paused: bool, cx: &mut Context<Self>) {
        if self.thread.goal.is_none() || (!paused && self.refuse_at_budget(cx)) {
            return;
        }
        self.thread.goal_status = Some(if paused {
            GoalStatus::Paused
        } else {
            GoalStatus::Active
        });
        if !paused {
            self.thread.goal_runs = 0;
        }
        self.save_thread(cx);
        if !paused && !self.running && self.preparing.is_none() {
            let prompt = self.goal_prompt(false);
            self.submit(Prompt::text(prompt), cx);
        }
        cx.notify();
    }

    pub(super) fn goal_prompt(&self, first: bool) -> String {
        let goal = self.thread.goal.as_deref().unwrap_or("");
        let lead = if first {
            "Work toward this goal across as many turns as it takes"
        } else {
            "Keep working toward the goal"
        };
        format!(
            "{lead}:\n\n{goal}\n\nTake the next concrete step and verify it. When the goal is fully achieved and verified, end your reply with the line {GOAL_DONE}. If you are blocked and need me, end with {GOAL_BLOCKED} and say what you need."
        )
    }

    /// After a turn: continue, finish or stop the goal.
    pub(super) fn continue_goal(&mut self, is_error: bool, cx: &mut Context<Self>) {
        if self.thread.goal_status != Some(GoalStatus::Active) {
            return;
        }
        let reply = self.last_reply();
        let budget = crate::preferences::Preferences::global(cx)
            .goal_max_turns
            .max(1);
        let (status, note) = if reply.contains(GOAL_DONE) {
            (GoalStatus::Achieved, "Goal achieved.".to_string())
        } else if let Some(budget) = self.budget_reached() {
            (
                GoalStatus::Paused,
                format!(
                    "Goal paused: the thread reached its {} budget.",
                    format_usd(budget)
                ),
            )
        } else if reply.contains(GOAL_BLOCKED) || is_error {
            (
                GoalStatus::Paused,
                "Goal paused: the agent needs you.".to_string(),
            )
        } else if self.thread.goal_runs >= budget {
            (
                GoalStatus::Exhausted,
                format!("Goal paused after {budget} automatic turns."),
            )
        } else {
            self.thread.goal_runs += 1;
            self.save_thread(cx);
            let prompt = self.goal_prompt(false);
            self.send(Prompt::text(prompt), cx);
            return;
        };
        self.thread.goal_status = Some(status);
        self.save_thread(cx);
        self.notice(note, false, cx);
        cx.emit(SessionEvent::NeedsAttention);
    }

    pub fn spent_usd(&self) -> f64 {
        self.items
            .iter()
            .filter_map(|item| match item.content {
                ItemContent::TurnSummary { cost_usd, .. } => cost_usd,
                _ => None,
            })
            // An empty f64 sum is -0.0, shown as "$-0.00".
            .fold(0.0, |total, cost| total + cost)
    }

    /// The budget, when the thread has spent all of it.
    pub fn budget_reached(&self) -> Option<f64> {
        self.thread
            .budget_usd
            .filter(|budget| self.spent_usd() >= *budget)
    }

    pub fn set_budget(&mut self, budget: Option<f64>, cx: &mut Context<Self>) {
        self.thread.budget_usd = budget.filter(|b| b.is_finite() && *b > 0.);
        self.save_thread(cx);
        cx.notify();
    }

    /// Say so, and return true, when the budget is used up.
    pub(super) fn refuse_at_budget(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(budget) = self.budget_reached() else {
            return false;
        };
        self.notice(
            format!(
                "This thread has spent {} of its {} budget. Raise the budget to let the goal continue.",
                format_usd(self.spent_usd()),
                format_usd(budget)
            ),
            false,
            cx,
        );
        true
    }

    /// After a turn: say when the thread passed 80% of its budget, or all of it.
    pub(super) fn check_budget(&mut self, spent_before: f64, cx: &mut Context<Self>) {
        let Some(budget) = self.thread.budget_usd else {
            return;
        };
        let spent = self.spent_usd();
        let note = if spent_before < budget && spent >= budget {
            format!(
                "Budget reached: {} of {} spent. Goals stop here; your own messages still go.",
                format_usd(spent),
                format_usd(budget)
            )
        } else if spent_before < budget * 0.8 && spent >= budget * 0.8 && spent < budget {
            format!(
                "{} of this thread's {} budget spent.",
                format_usd(spent),
                format_usd(budget)
            )
        } else {
            return;
        };
        self.notice(note, false, cx);
        cx.emit(SessionEvent::NeedsAttention);
    }
}
