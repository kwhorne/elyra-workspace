//! What agents propose for the user to keep: automations, rules, checks
//! (invariants), and updates to pull requests the thread reviewed.

use super::*;

impl ThreadSession {
    /// Show an automation the agent proposed, for the user to decide on.
    pub fn propose_automation(
        &mut self,
        automation: elyra_core::Automation,
        cx: &mut Context<Self>,
    ) {
        self.append(
            ItemContent::AutomationProposal {
                automation: Box::new(automation),
                outcome: None,
            },
            cx,
        );
        cx.notify();
    }

    /// A pull request the thread reviewed changed (see `review_follow`).
    pub fn add_pr_update(&mut self, card: ItemContent, cx: &mut Context<Self>) {
        self.append(card, cx);
        cx.notify();
    }

    pub fn set_pr_update_handled(&mut self, item: ItemId, cx: &mut Context<Self>) {
        let Some(index) = self.items.iter().position(|i| i.id == item) else {
            return;
        };
        if let ItemContent::PrUpdate { handled, .. } = &mut self.items[index].content {
            *handled = true;
            self.persist_item(index, cx);
            cx.notify();
        }
    }

    /// Show an invariant the agent proposed, and try it on the code as it is.
    pub fn propose_invariant(&mut self, invariant: elyra_core::Invariant, cx: &mut Context<Self>) {
        self.append(
            ItemContent::InvariantProposal {
                invariant: invariant.clone(),
                holds: None,
                outcome: None,
            },
            cx,
        );
        let Some(card) = self.items.last().map(|item| item.id) else {
            return;
        };
        let dir = self.working_dir();
        let job = cx.background_executor().spawn(async move {
            let root = elyra_git::repo_root(&dir).unwrap_or(dir);
            crate::invariants::try_on(&root, &invariant)
        });
        cx.spawn(async move |this, cx| {
            let found = job.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(index) = this.items.iter().position(|item| item.id == card)
                    && let ItemContent::InvariantProposal { holds, .. } =
                        &mut this.items[index].content
                {
                    *holds = Some(found);
                    this.persist_item(index, cx);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Save a proposed invariant in the repository, or dismiss it.
    pub fn decide_invariant(&mut self, card: ItemId, save: bool, cx: &mut Context<Self>) {
        let Some(index) = self.items.iter().position(|item| item.id == card) else {
            return;
        };
        let ItemContent::InvariantProposal {
            invariant, outcome, ..
        } = &self.items[index].content
        else {
            return;
        };
        if outcome.is_some() {
            return;
        }
        let invariant = invariant.clone();
        let decided = if save {
            let dir = self.working_dir();
            let root = elyra_git::repo_root(&dir).unwrap_or(dir);
            match crate::invariants::save(&root, &invariant) {
                Ok(path) => {
                    let path = path
                        .strip_prefix(&root)
                        .unwrap_or(&path)
                        .display()
                        .to_string();
                    self.notice(
                        format!("Saved as a check in {path}; it runs after every turn that changes the files it covers. Commit it with your changes."),
                        false,
                        cx,
                    );
                    elyra_core::ProposalOutcome::Created
                }
                Err(err) => {
                    self.notice(format!("Couldn't save the check: {err:#}"), true, cx);
                    return;
                }
            }
        } else {
            elyra_core::ProposalOutcome::Dismissed
        };
        let Some(index) = self.items.iter().position(|item| item.id == card) else {
            return;
        };
        if let ItemContent::InvariantProposal { outcome, .. } = &mut self.items[index].content {
            *outcome = Some(decided);
            self.persist_item(index, cx);
        }
        cx.notify();
    }

    /// Ask the agent to turn a proposed rule into a check it can propose.
    pub fn rule_as_check(&mut self, item: ItemId, cx: &mut Context<Self>) {
        let Some((rule, paths)) = self.proposed_rule(item) else {
            return;
        };
        let paths = paths
            .map(|p| format!(" It applies to {p}."))
            .unwrap_or_default();
        self.submit(
            Prompt::text(format!(
                "Turn the rule “{rule}” into a check Elyra can run after every turn, with the gateway's propose_invariant tool.{paths} Use `require` (a regular expression each of those files must have), `forbid` (one new lines mustn't have) or a short shell `command` over the files in ELYRA_FILES, whichever catches a mistake against it most simply. If it can't be checked mechanically, say so instead."
            )),
            cx,
        );
    }

    /// Show a rule the agent learned, for the user to keep or dismiss.
    pub fn propose_rule(
        &mut self,
        rule: String,
        reason: String,
        paths: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.append(
            ItemContent::RuleProposal {
                rule,
                reason,
                paths,
                outcome: None,
            },
            cx,
        );
        cx.notify();
    }

    /// The rule a proposal item holds, and the files it applies to.
    pub fn proposed_rule(&self, item: ItemId) -> Option<(String, Option<String>)> {
        self.items
            .iter()
            .find(|i| i.id == item)
            .and_then(|i| match &i.content {
                ItemContent::RuleProposal { rule, paths, .. } => {
                    Some((rule.clone(), paths.clone()))
                }
                _ => None,
            })
    }

    pub fn set_rule_outcome(
        &mut self,
        item: ItemId,
        outcome: elyra_core::RuleOutcome,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.items.iter().position(|i| i.id == item) else {
            return;
        };
        if let ItemContent::RuleProposal {
            outcome: current, ..
        } = &mut self.items[index].content
        {
            *current = Some(outcome);
            self.persist_item(index, cx);
            cx.notify();
        }
    }

    /// The automation a proposal item holds.
    pub fn proposal(&self, item: ItemId) -> Option<elyra_core::Automation> {
        self.items
            .iter()
            .find(|i| i.id == item)
            .and_then(|i| match &i.content {
                ItemContent::AutomationProposal { automation, .. } => Some((**automation).clone()),
                _ => None,
            })
    }

    pub fn set_proposal_outcome(
        &mut self,
        item: ItemId,
        outcome: elyra_core::ProposalOutcome,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.items.iter().position(|i| i.id == item) else {
            return;
        };
        if let ItemContent::AutomationProposal {
            outcome: current, ..
        } = &mut self.items[index].content
        {
            *current = Some(outcome);
            self.persist_item(index, cx);
            cx.notify();
        }
    }
}
