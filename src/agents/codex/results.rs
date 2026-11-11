//! What a finished MCP or dynamic tool call of a paginated rollout hands back. Claude Code's
//! transcript keeps every tool result, so a pull request an MCP server opened is found in it; the
//! Codex item keeps the result in a field of its own, and this reads the text out of it.

use crate::agents::outputs::RESULT_SCAN_CAP;
use serde_json::Value;

/// The deepest a result is searched for text: a content list inside a result inside an item.
const DEPTH: usize = 6;

/// The text of `item`'s result: every string in the first of `fields` it has, in order, a line
/// each, apart from the `type` tags. MCP calls keep it in `result`
/// (`{content: [{type: "text", text}], structured_content}`), dynamic tool calls in
/// `content_items`. The call's own `arguments` are never read.
pub fn text(item: &Value, fields: &[&str]) -> String {
    let mut out = String::new();
    if let Some(v) = fields.iter().find_map(|f| item.get(*f)) {
        collect(v, DEPTH, &mut out);
    }
    out
}

fn collect(v: &Value, depth: usize, out: &mut String) {
    if depth == 0 || out.len() >= RESULT_SCAN_CAP {
        return;
    }
    match v {
        Value::String(s) => {
            out.push_str(s);
            out.push('\n');
        }
        Value::Array(a) => a.iter().for_each(|x| collect(x, depth - 1, out)),
        Value::Object(o) => o
            .iter()
            .filter(|(k, _)| *k != "type")
            .for_each(|(_, x)| collect(x, depth - 1, out)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_text_of_a_result_is_every_string_in_it() {
        let item = json!({
            "arguments": {"title": "not read"},
            "result": {"content": [{"type": "text", "text": "opened https://github.com/org/app/pull/7"}],
                       "structured_content": {"links": ["https://acme.atlassian.net/browse/APP-9"]}}
        });
        let t = text(&item, &["result"]);
        assert!(t.contains("pull/7") && t.contains("browse/APP-9"));
        assert!(!t.contains("not read"));
    }

    #[test]
    fn the_first_field_the_item_has_is_the_one_read() {
        let item = json!({"content_items": [{"type": "inputText", "text": "done"}]});
        assert_eq!(text(&item, &["result", "content_items"]), "done\n");
        assert_eq!(text(&json!({"status": "failed"}), &["result"]), "");
        assert_eq!(text(&json!({"result": null}), &["result"]), "");
    }

    #[test]
    fn a_huge_result_is_cut_and_a_deep_one_is_not_followed() {
        let big = json!({"result": {"content": vec![json!("x".repeat(10_000)); 20]}});
        let t = text(&big, &["result"]);
        assert!(t.len() < RESULT_SCAN_CAP + 10_001, "{}", t.len());
        let mut deep = json!("https://github.com/org/app/pull/1");
        for _ in 0..10 {
            deep = json!([deep]);
        }
        assert_eq!(text(&json!({"result": deep}), &["result"]), "");
    }
}
