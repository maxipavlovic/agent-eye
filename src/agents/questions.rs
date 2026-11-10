//! Does a reply ask the user something? The adapters use it to tell a session that finished from
//! one that waits for an answer.

/// Markup and quotes that can follow a question mark: `**Ready to merge?**`.
const AFTER_QUESTION: [char; 7] = ['*', '_', ')', '"', '\'', '`', ' '];

/// Does the reply end in a question?
pub fn asks_user(tail: &str) -> bool {
    tail.trim_end()
        .trim_end_matches(AFTER_QUESTION)
        .ends_with('?')
}

/// The last sentence of a reply that ends in a question: the question.
pub fn last_question(tail: &str) -> String {
    let t = tail.trim_end().trim_end_matches(AFTER_QUESTION);
    let body = &t[..t.len() - 1];
    let start = body.rfind(['.', '!', '?', '\n', ':']).map_or(0, |i| i + 1);
    let question = t[start..].split_whitespace().collect::<Vec<_>>().join(" ");
    question.trim_start_matches(AFTER_QUESTION).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn question_detection() {
        assert!(asks_user("Shall I go on?"));
        assert!(asks_user("**Ready to merge?**"));
        assert!(!asks_user("Done."));
        assert_eq!(
            last_question("Done. **Ready to merge?**"),
            "Ready to merge?"
        );
        assert_eq!(last_question("Shall I go on?"), "Shall I go on?");
    }
}
