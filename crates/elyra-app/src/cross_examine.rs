//! Cross-examine a plan: before any code is written, another agent reads the
//! plan in a read-only side chat, checks it against the code (callers, cases
//! it misses, what it should reuse, a simpler way) and answers with concrete
//! objections only. The user takes or leaves each one, and the planning agent
//! revises the plan with the ones taken. A fraction of what racing whole
//! implementations costs, and the disagreement comes before the code.

use elyra_core::Objection;
use serde_json::Value;

/// How the examiner's message starts (and how it is recognised).
pub const PROMPT_START: &str = "You are cross-examining a plan";
/// How the message starts that sends the objections taken back.
pub const REVISE_START: &str = "Another agent cross-examined the plan";

const MAX_OBJECTIONS: usize = 6;

/// The examiner's message: the request, the plan, an orientation from the
/// symbol index (warm start), and what to look for.
pub fn prompt(author: &str, request: &str, plan: &str, orientation: Option<&str>) -> String {
    let orientation = orientation
        .map(|text| format!("\n{text}\n"))
        .unwrap_or_default();
    format!(
        "{PROMPT_START} that another agent ({author}) wrote, before any code is written. \
         You are read-only: read and search the code (and use the symbol tools when you have \
         them), change nothing.\n\n\
         The request:\n\"\"\"\n{request}\n\"\"\"\n\n\
         The plan:\n\"\"\"\n{plan}\n\"\"\"\n{orientation}\n\
         Check the plan against the code and look for what it gets wrong or misses:\n\
         - callers or other uses of what it changes that it doesn't account for (look them up);\n\
         - cases it misses: errors, empty or missing values, permissions, existing data;\n\
         - code that already does part of it and should be reused;\n\
         - a clearly simpler way to the same result.\n\n\
         Only concrete objections you have checked in the code, at most 5, the most important \
         first. No style points, no restating the plan, no praise. If the plan holds up, give an \
         empty list.\n\n\
         End your reply with this JSON block and nothing after it:\n\
         ```json\n\
         {{\"objections\": [{{\"point\": \"what is wrong, in one line\", \"why\": \"the evidence: files, lines, callers\", \"instead\": \"what to do instead\"}}]}}\n\
         ```"
    )
}

/// The objections in the examiner's reply: the last JSON block, or failing
/// that the last `{…}` in it. None when there is no list to read.
pub fn parse(reply: &str) -> Option<Vec<Objection>> {
    let block = reply.rfind("```json").and_then(|start| {
        let body = &reply[start + "```json".len()..];
        body.find("```").map(|end| &body[..end])
    });
    let span = |open: char, close: char| {
        let (start, end) = (reply.find(open)?, reply.rfind(close)?);
        serde_json::from_str::<Value>(reply.get(start..=end)?).ok()
    };
    let value = block
        .and_then(|b| serde_json::from_str::<Value>(b.trim()).ok())
        .or_else(|| {
            // Without the fence: whichever of an object or a list comes first.
            match (reply.find('['), reply.find('{')) {
                (Some(list), Some(object)) if list < object => span('[', ']'),
                _ => span('{', '}').or_else(|| span('[', ']')),
            }
        })?;
    let list = match &value {
        Value::Array(list) => list,
        other => other.get("objections")?.as_array()?,
    };
    let text = |value: &Value, key: &str, max: usize| {
        let text = value[key].as_str().unwrap_or("").trim();
        if text.chars().count() > max {
            format!("{}…", text.chars().take(max).collect::<String>())
        } else {
            text.to_string()
        }
    };
    Some(
        list.iter()
            .map(|item| Objection {
                point: text(item, "point", 300),
                why: text(item, "why", 1200),
                instead: text(item, "instead", 800),
                taken: None,
            })
            .filter(|objection| !objection.point.is_empty())
            .take(MAX_OBJECTIONS)
            .collect(),
    )
}

/// What goes back to the planning agent: the objections taken, and those
/// the user considered and kept the plan on.
pub fn revision(examiner: &str, objections: &[Objection]) -> Option<String> {
    let taken: Vec<&Objection> = objections
        .iter()
        .filter(|o| o.taken == Some(true))
        .collect();
    if taken.is_empty() {
        return None;
    }
    let mut text = format!(
        "{REVISE_START} ({examiner}) before any code was written. The user agrees with these \
         points: revise the plan to deal with them, then present it again.\n"
    );
    for (n, objection) in taken.iter().enumerate() {
        text.push_str(&format!("\n{}. {}", n + 1, objection.point));
        if !objection.why.is_empty() {
            text.push_str(&format!("\n   Why: {}", objection.why));
        }
        if !objection.instead.is_empty() {
            text.push_str(&format!("\n   Instead: {}", objection.instead));
        }
    }
    let kept: Vec<&Objection> = objections
        .iter()
        .filter(|o| o.taken == Some(false))
        .collect();
    if !kept.is_empty() {
        text.push_str("\n\nThe user keeps the plan as it is on these, so leave them:\n");
        for objection in kept {
            text.push_str(&format!("- {}\n", objection.point));
        }
    }
    Some(text.trim_end().to_string())
}

#[cfg(test)]
mod tests {
    use super::{PROMPT_START, parse, prompt, revision};

    #[test]
    fn asks_for_checked_objections_only() {
        let text = prompt(
            "Claude Code",
            "Add a discount code to checkout",
            "1. Add `discount` to CartTotal",
            Some("Where the names in it are defined: CartTotal (app/Cart/Total.php:12)"),
        );
        assert!(text.starts_with(PROMPT_START));
        assert!(text.contains("1. Add `discount` to CartTotal"));
        assert!(text.contains("app/Cart/Total.php:12"));
        assert!(text.contains("```json"));
    }

    #[test]
    fn reads_the_last_json_block() {
        let reply = "I looked at the callers.\n\n```json\n{\"objections\": [\
            {\"point\": \"CartTotal is also used by the invoice PDF\", \"why\": \"app/Pdf/Invoice.php:40 calls total()\", \"instead\": \"Add discount() next to total()\"},\
            {\"point\": \"\", \"why\": \"empty points are dropped\"},\
            {\"point\": \"Codes are case-sensitive in the plan\"}]}\n```\n";
        let found = parse(reply).unwrap();
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].point, "CartTotal is also used by the invoice PDF");
        assert_eq!(found[0].instead, "Add discount() next to total()");
        assert_eq!(found[1].why, "");
        assert_eq!(
            parse("```json\n{\"objections\": []}\n```"),
            Some(Vec::new())
        );
        // Without the fence, and as a bare list.
        assert_eq!(
            parse("{\"objections\": [{\"point\": \"x\"}]}")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(parse("[{\"point\": \"x\"}]").unwrap().len(), 1);
        assert_eq!(parse("The plan looks fine to me."), None);
    }

    #[test]
    fn sends_back_what_was_taken() {
        let mut objections = parse(
            "```json\n{\"objections\": [{\"point\": \"Misses the invoice PDF\", \"why\": \"Invoice.php:40\", \"instead\": \"Add discount()\"}, {\"point\": \"Use a service class\"}, {\"point\": \"Not looked at\"}]}\n```",
        )
        .unwrap();
        assert_eq!(revision("Codex", &objections), None, "nothing taken yet");
        objections[0].taken = Some(true);
        objections[1].taken = Some(false);
        let text = revision("Codex", &objections).unwrap();
        assert!(text.starts_with(super::REVISE_START));
        assert!(text.contains(
            "1. Misses the invoice PDF\n   Why: Invoice.php:40\n   Instead: Add discount()"
        ));
        assert!(text.contains("leave them:\n- Use a service class"));
        assert!(!text.contains("Not looked at"));
    }
}
