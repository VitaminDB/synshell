use std::ops::Range;

use crate::chat::{ChatMsg, Role};

pub const SUMMARY_PROMPT: &str = "You compress a dialogue fragment into a brief technical description: key decisions, file names, tool-call outcomes, open questions. Keep the specifics (numbers, identifiers, names, paths). Don't retell verbatim. Write neutrally, in 5-12 short paragraphs, in the language of the dialogue.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    BetweenTurns,
    InTurn { keep_tail: usize },
}

fn live(m: &ChatMsg) -> bool {
    !m.compacted && m.role != Role::Summary
}

fn last_user(msgs: &[ChatMsg]) -> Option<usize> {
    msgs.iter().rposition(|m| m.role == Role::User && !m.compacted)
}

pub fn find_range(msgs: &[ChatMsg], scope: Scope) -> Option<Range<usize>> {
    let pivot = last_user(msgs)?;
    let (mut start, mut end) = match scope {
        Scope::BetweenTurns => (0, pivot),
        Scope::InTurn { keep_tail } => (pivot + 1, msgs.len().checked_sub(keep_tail)?),
    };
    while start < end && !live(&msgs[start]) {
        start += 1;
    }
    while start < end && msgs[start].role == Role::Tool {
        start += 1;
    }
    while end > start && end < msgs.len() && msgs[end].role == Role::Tool {
        end -= 1;
    }
    if end <= start || msgs[start..end].iter().filter(|m| live(m)).count() < 2 {
        return None;
    }
    Some(start..end)
}

fn cut(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}… [обрезано]")
}

pub fn serialize(msgs: &[ChatMsg], range: Range<usize>) -> String {
    const MAX_BODY: usize = 1500;
    const MAX_TOTAL: usize = 16 * 1024;
    let mut buf = String::new();
    for m in &msgs[range] {
        if m.compacted {
            continue;
        }
        match m.role {
            Role::Summary => buf.push_str(&format!("[Earlier summary]: {}\n\n", m.text.trim())),
            Role::User => buf.push_str(&format!("User: {}\n\n", cut(&m.text, MAX_BODY))),
            Role::Assistant => {
                if !m.text.trim().is_empty() {
                    buf.push_str(&format!("Assistant: {}\n", cut(&m.text, MAX_BODY)));
                }
                for c in &m.tool_calls {
                    buf.push_str(&format!("Assistant called tool `{}`(args={}).\n", c.name, cut(&c.arguments, 400)));
                }
                buf.push('\n');
            }
            Role::Tool => {
                let (name, ok) = m.tool.as_ref().map(|t| (t.name.as_str(), t.ok)).unwrap_or(("tool", true));
                buf.push_str(&format!("Tool `{name}` ({}): {}\n\n", if ok { "ok" } else { "error" }, cut(&m.text, MAX_BODY)));
            }
        }
        if buf.chars().count() > MAX_TOTAL {
            buf.push_str("\n[…history truncated for the summary request…]\n");
            break;
        }
    }
    buf
}

pub fn apply(msgs: &mut Vec<ChatMsg>, range: Range<usize>, summary: String) -> usize {
    let mut n = 0;
    for m in &mut msgs[range.clone()] {
        if !m.compacted {
            m.compacted = true;
            n += 1;
        }
    }
    msgs.insert(range.start, ChatMsg::summary(summary, n));
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use synaptixd::ToolCall;

    fn call() -> ChatMsg {
        ChatMsg::assistant("", String::new(), vec![ToolCall { name: "bash".into(), arguments: "{}".into() }])
    }

    fn tool() -> ChatMsg {
        ChatMsg::tool("bash", "$ ls".into(), true, "a b".into(), Vec::new())
    }

    #[test]
    fn between_turns_keeps_last_request() {
        let mut msgs = vec![ChatMsg::user("q1"), call(), tool(), ChatMsg::assistant("a1", String::new(), Vec::new()), ChatMsg::user("q2")];
        let r = find_range(&msgs, Scope::BetweenTurns).unwrap();
        assert_eq!(r, 0..4);
        let text = serialize(&msgs, r.clone());
        assert!(text.contains("User: q1") && text.contains("Tool `bash` (ok): a b"), "{text}");
        assert_eq!(apply(&mut msgs, r, "сводка".into()), 4);
        assert_eq!(msgs[0].role, Role::Summary);
        assert!(find_range(&msgs, Scope::BetweenTurns).is_none());
    }

    #[test]
    fn in_turn_never_splits_tool_results() {
        let msgs = vec![ChatMsg::user("q"), call(), tool(), call(), tool(), tool(), call(), tool()];
        let r = find_range(&msgs, Scope::InTurn { keep_tail: 3 }).unwrap();
        assert_eq!(r, 1..3);
        assert_eq!(find_range(&msgs, Scope::InTurn { keep_tail: 2 }).unwrap(), 1..6);
        assert!(find_range(&msgs[..3], Scope::InTurn { keep_tail: 1 }).is_none());
    }
}
