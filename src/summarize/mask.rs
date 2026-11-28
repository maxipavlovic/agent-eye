//! Masking: take the secrets out of text before it goes to a summary agent, an API reply or a
//! page. Best effort: a secret with no name and no known prefix is not found.

use regex::Regex;
use std::sync::LazyLock;

static MASKS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    let m = |p: &str, r: &'static str| (Regex::new(p).expect("static regex"), r);
    vec![
        m(
            r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?(?:-----END [A-Z ]*PRIVATE KEY-----|$)",
            "[masked]",
        ),
        m(r"AKIA[0-9A-Z]{16}", "[masked]"),
        m(r"gh[pousr]_[A-Za-z0-9]{36,}", "[masked]"),
        m(r"xox[abpr]-[A-Za-z0-9-]{10,}", "[masked]"),
        m(r"eyJ[\w-]+\.[\w-]+\.[\w-]+", "[masked]"),
        m(r"Bearer\s+[A-Za-z0-9._-]{16,}", "Bearer [masked]"),
        // keys that give no name: the provider's own prefix is the only sign
        m(
            r"\bsk-(?:(?:ant|proj|svcacct|admin)-[A-Za-z0-9_-]{16,}|[A-Za-z0-9]{20,})",
            "[masked]",
        ),
        m(r"AIza[0-9A-Za-z_-]{35,}", "[masked]"),
        m(r"glpat-[A-Za-z0-9_-]{20,}", "[masked]"),
        m(r"github_pat_\w{22,}", "[masked]"),
        m(r"npm_[A-Za-z0-9]{36}", "[masked]"),
        // only behind a header name: a bare "Basic" is also an ordinary word
        m(
            r#"(?i)(authorization["']?\s*[:=]\s*["']?)Basic\s+[A-Za-z0-9+/]{8,}={0,2}"#,
            "${1}Basic [masked]",
        ),
        // a password in the address: `https://user:password@host/`
        m(r"(://[^/\s:@]+:)[^@\s/]+@", "${1}[masked]@"),
        // a key and its value, quoted or not (`"token": "..."`, `aws_secret_access_key = ...`)
        m(
            r#"(?i)(api[_-]?key|secret(?:[_-]?access)?(?:[_-]?key)?|token|password|passwd|authorization)(["']?\s*[:=]\s*)(["']?)[^\s"']{8,}"#,
            "$1$2$3[masked]",
        ),
        // a flag and its value (`--token abc...`)
        m(
            r#"(?i)(--[\w-]*(?:api[_-]?key|secret|token|password|passwd))(\s+)(["']?)[^\s"']{8,}"#,
            "$1$2$3[masked]",
        ),
    ]
});

/// Replace API keys, tokens, passwords and private keys with `[masked]` (best effort).
pub fn mask_secrets(text: &str) -> String {
    let mut t = text.to_string();
    for (re, rep) in MASKS.iter() {
        t = re.replace_all(&t, *rep).into_owned();
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_secrets() {
        // made-up secrets, assembled at run time so no token-shaped literal sits in the code
        let gh = format!("{}_{}", "ghp", "abcdefghijklmnopqrstuvwxyzABCDEFGHIJ12");
        let t = &format!(
            "key AKIAABCDEFGHIJKLMNOP and {gh} and xoxb-1234567890-abc \
                 jwt eyJhbGciOi.eyJzdWIiOi.SflKxwRJSM Authorization: Bearer abcdefghijklmnop12345 \
                 password=hunter2hunter2 api_key: \"sk12345678\" short token=abc\n\
                 -----BEGIN RSA PRIVATE KEY-----\nMIIE\nabc\n-----END RSA PRIVATE KEY-----\nafter"
        );
        let m = mask_secrets(t);
        for bad in [
            "AKIAABCDEFGHIJKLMNOP",
            "ghp_abc",
            "xoxb-1234",
            "eyJhbGci",
            "abcdefghijklmnop12345",
            "hunter2hunter2",
            "sk12345678",
            "MIIE",
        ] {
            assert!(!m.contains(bad), "{bad} leaked in {m}");
        }
        assert!(m.contains("password=[masked]"));
        assert!(m.contains("token=abc"));
        assert!(m.contains("after"));
    }

    /// The shapes a secret takes when an agent echoes a file or a command (`cat config.json`,
    /// `curl -H`, a clone URL): found in review probes of 0.5.0-rc.3, which left every one of them
    /// as it was. Made-up values, split so that no token-shaped literal sits in the code.
    #[test]
    fn masks_the_shapes_a_secret_takes_in_a_transcript() {
        let letters = "AbCdEfGhIjKlMnOpQrStUvWxYz";
        let cases: [(String, &str, &str); 12] = [
            (
                format!("key sk-ant-api03-{letters}0123456789abcdefghij"),
                letters,
                "key [masked]",
            ),
            (
                format!("use sk-proj-{letters}0123456789abcd"),
                letters,
                "use [masked]",
            ),
            (
                format!("{}Sy{letters}0123456789", "AIza"),
                letters,
                "[masked]",
            ),
            (format!("{}-{letters}", "glpat"), letters, "[masked]"),
            (
                format!("{}_11ABCDEFG0{letters}0123456789", "github_pat"),
                letters,
                "[masked]",
            ),
            (
                format!("{}_{letters}0123456789", "npm"),
                letters,
                "[masked]",
            ),
            (
                "Authorization: Basic dXNlcjpwYXNzd29yZDEyMzQ1Njc4".into(),
                "dXNlcjpw",
                "Authorization: Basic [masked]",
            ),
            (
                format!("curl --token {letters} https://x"),
                letters,
                "curl --token [masked] https://x",
            ),
            (
                "git clone https://user:s3cretpassw0rd@host/repo.git".into(),
                "s3cretpassw0rd",
                "https://user:[masked]@host/repo.git",
            ),
            (
                format!("export ANTHROPIC_API_KEY=sk-ant-api03-{letters}"),
                letters,
                "ANTHROPIC_API_KEY=[masked]",
            ),
            (
                format!("{{\"token\": \"{letters}\"}}"),
                letters,
                "{\"token\": \"[masked]\"}",
            ),
            (
                "aws_secret_access_key = wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
                "wJalrXUtnFEMI",
                "aws_secret_access_key = [masked]",
            ),
        ];
        for (text, secret, kept) in cases {
            let m = mask_secrets(&text);
            assert!(!m.contains(&secret), "{secret} leaked: {m}");
            assert!(m.contains(kept), "{kept:?} missing: {m}");
        }
    }

    /// What looks a little like a secret and is not: the rules must leave it as it is.
    #[test]
    fn ordinary_code_hashes_and_addresses_stay_as_they_are() {
        for text in [
            "commit 9fceb02d0ae598e95dc970b74767f19372d61af8",
            "sha256:3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a",
            "bbbbbbbb-0000-4000-8000-000000000001",
            "fn mask(token: &str) -> String { let secret_len = 12; }",
            "max_tokens: 4096, tokenizer: fast",
            "the task-management-and-risk-assessment-framework page",
            "https://example.com:8080/path and http://localhost:3000/a@b",
            "git clone git@github.com:org/repo.git, ssh://git@host/repo.git",
            "Basic configuration steps, Basic internationalization",
            "npm_lifecycle_event=build npm_package_version=1.2.3",
            "AIzaSy is how those keys begin, glpat- too, and sk- and github_pat_",
        ] {
            assert_eq!(mask_secrets(text), text);
        }
    }
}
