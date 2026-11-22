//! Reading what was typed: whitespace-separated units, each one a word, a phrase or a link.

/// One thing the user typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unit {
    /// The tokens of the unit, lowercase (`cloud.im` is `cloud`, `im`).
    pub parts: Vec<String>,
    /// The unit as typed, lowercase; ideas are matched on it as a substring.
    pub needle: String,
    /// The user may still be typing it: match words that start with its last token.
    pub prefix: bool,
    /// A single word that may be fixed when nothing matches it.
    pub typo_ok: bool,
}

impl Unit {
    /// The FTS5 phrase of the unit (`"cloud im"`, with a `*` after it for a prefix).
    pub fn phrase(&self) -> String {
        format!(
            "\"{}\"{}",
            self.parts.join(" "),
            if self.prefix { "*" } else { "" }
        )
    }
}

/// A parsed search.
#[derive(Debug, Clone, Default)]
pub struct Query {
    pub units: Vec<Unit>,
    /// The number of words typed, which sets how long a title may be before it counts as longer.
    pub words: usize,
}

impl Query {
    /// True when nothing searchable was typed (empty, or only punctuation).
    pub fn is_empty(&self) -> bool {
        self.units.is_empty()
    }

    /// How many units a row must match: half of them, rounded up.
    pub fn need(&self) -> usize {
        self.units.len().div_ceil(2)
    }
}

/// The tokens of a text, as the FTS tokenizer sees them: runs of letters and digits, lowercase.
pub fn tokens(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Where a link points: the path segments worth searching. The scheme, host, query and fragment
/// are dropped. A segment of 8+ characters with a digit (an artifact id, a ticket key) is the
/// link's name: when there is one, only those segments are kept.
fn link_segments(link: &str) -> Vec<String> {
    let rest = link.split_once("://").map_or(link, |(_, r)| r);
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let segs: Vec<&str> = rest.split('/').skip(1).filter(|s| !s.is_empty()).collect();
    let named: Vec<&str> = segs
        .iter()
        .copied()
        .filter(|s| s.chars().count() >= 8 && s.chars().any(|c| c.is_ascii_digit()))
        .collect();
    let keep = if named.is_empty() { segs } else { named };
    keep.into_iter().map(str::to_lowercase).collect()
}

fn is_link(w: &str) -> bool {
    let w = w.to_lowercase();
    w.starts_with("http://") || w.starts_with("https://")
}

/// The most units a search reads; the rest of a long paste is dropped.
const MAX_UNITS: usize = 20;

/// Parse the words of a search. At most the first 20 units are kept. Only the last unit is a prefix match, and only when its last
/// token has 3+ characters; the rest match whole words (stemming covers word forms).
pub fn parse(words: &[String]) -> Query {
    let mut units = vec![];
    let mut typed = 0;
    let mut last_link = false;
    for w in words.iter().flat_map(|w| w.split_whitespace()) {
        if units.len() >= MAX_UNITS {
            break;
        }
        if w.chars().any(char::is_alphanumeric) {
            typed += 1;
        }
        let (texts, link) = if is_link(w) {
            (link_segments(w), true)
        } else {
            (vec![w.to_lowercase()], false)
        };
        for text in texts {
            let parts = tokens(&text);
            if parts.is_empty() {
                continue;
            }
            let typo_ok = !link && parts.len() == 1 && parts[0].chars().count() >= 4;
            last_link = link;
            units.push(Unit {
                parts,
                needle: text,
                prefix: false,
                typo_ok,
            });
        }
    }
    // the last unit is still being typed, unless it came from a link
    units.truncate(MAX_UNITS);
    if let (Some(u), false) = (units.last_mut(), last_link) {
        u.prefix = u.parts.last().is_some_and(|t| t.chars().count() >= 3);
    }
    Query {
        units,
        words: typed,
    }
}
