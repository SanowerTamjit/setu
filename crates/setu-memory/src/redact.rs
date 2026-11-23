//! Masks secrets in a command line before it is kept.
//!
//! This is the second layer: the terminal already skips lines typed at a
//! password prompt or without echo. Here we look at the words of what is
//! left and mask anything that looks like a secret:
//!
//! - `NAME=value` where the name is about a password, token, secret or key
//!   (`DB_PASSWORD=…`, `--token=…`, `export GITHUB_TOKEN=…`);
//! - the word after a flag like that (`--password x`, `--api-key x`);
//! - `user:password@` in URLs, `curl -u user:password`,
//!   `Authorization: Bearer …`, `mysql -pPASSWORD`, `sshpass -p …`;
//! - words shaped like well-known tokens (GitHub, GitLab, Slack, AWS
//!   access key ids, JWTs, `sk-…` API keys) or long random strings.
//!
//! It masks too much rather than too little: a masked command is still
//! listed (so the person sees what was kept) but never suggested.

/// What replaces a secret.
pub const MASK: &str = "[redacted]";

/// A command after masking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redacted {
    pub text: String,
    /// Whether anything was masked.
    pub masked: bool,
}

/// Masks secrets in `command`. Returns `None` when the whole line must not
/// be kept at all (it holds a private key).
pub fn redact(command: &str) -> Option<Redacted> {
    if command.contains("PRIVATE KEY") || command.contains("-----BEGIN") {
        return None;
    }
    let spans = words(command);
    let program = program_of(command, &spans);
    let mut out = String::with_capacity(command.len());
    let mut masked = false;
    let mut last = 0;
    let mut prev: Option<&str> = None;
    for &(start, end) in &spans {
        out.push_str(&command[last..start]);
        let word = &command[start..end];
        let replaced = mask_word(word, prev, program);
        if let Some(r) = replaced {
            masked = true;
            out.push_str(&r);
        } else {
            out.push_str(word);
        }
        last = end;
        prev = Some(word);
    }
    out.push_str(&command[last..]);
    Some(Redacted { text: out, masked })
}

/// Byte ranges of the whitespace-separated words.
fn words(s: &str) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();
    let mut start = None;
    for (i, c) in s.char_indices() {
        if c.is_whitespace() {
            if let Some(st) = start.take() {
                spans.push((st, i));
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(st) = start {
        spans.push((st, s.len()));
    }
    spans
}

/// The program being run: the first word that is not `VAR=x` or `sudo`.
fn program_of<'a>(command: &'a str, spans: &[(usize, usize)]) -> &'a str {
    for &(s, e) in spans {
        let w = &command[s..e];
        if w.contains('=') || w == "sudo" || w == "env" || w == "exec" || w.starts_with('-') {
            continue;
        }
        return w.rsplit('/').next().unwrap_or(w);
    }
    ""
}

fn unquote(w: &str) -> &str {
    w.trim_matches(|c| c == '"' || c == '\'' || c == '`')
}

/// Whether a variable or flag name is about a secret.
fn sensitive_name(name: &str) -> bool {
    let upper = name
        .trim_start_matches('-')
        .trim_start_matches("export ")
        .to_ascii_uppercase();
    if upper.is_empty() {
        return false;
    }
    let joined: String = upper
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    const JOINED: &[&str] = &[
        "APIKEY",
        "ACCESSKEY",
        "PRIVATEKEY",
        "SECRETKEY",
        "AUTHKEY",
        "CLIENTSECRET",
    ];
    if JOINED.iter().any(|j| joined.contains(j)) {
        return true;
    }
    const WORDS: &[&str] = &[
        "PASS",
        "PASSWORD",
        "PASSWD",
        "PASSPHRASE",
        "TOKEN",
        "SECRET",
        "CREDENTIAL",
        "CREDENTIALS",
        "PAT",
        "PIN",
        "OTP",
        "APIKEY",
        "AUTH",
        "COOKIE",
    ];
    upper
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|part| WORDS.contains(&part) || part.ends_with("PASSWORD") || part.ends_with("TOKEN"))
}

fn mask_word(word: &str, prev: Option<&str>, program: &str) -> Option<String> {
    // user:password@ in a URL.
    if let Some(r) = mask_url(word) {
        return Some(r);
    }
    // NAME=value and --name=value.
    if let Some(eq) = word.find('=') {
        let name = unquote(&word[..eq]);
        let value = &word[eq + 1..];
        if !name.is_empty() && !unquote(value).is_empty() && sensitive_name(name) {
            return Some(format!("{}={}", &word[..eq], keep_quotes(value)));
        }
    }
    let bare = unquote(word);
    if let Some(prev) = prev.map(unquote) {
        let lower = prev.to_ascii_lowercase();
        // --password secret
        if lower.starts_with('-')
            && !lower.contains('=')
            && sensitive_name(&lower)
            && !bare.starts_with('-')
        {
            return Some(keep_quotes(word));
        }
        // Authorization: Bearer secret
        if (lower == "bearer" || lower == "basic" || lower == "token") && !bare.is_empty() {
            return Some(keep_quotes(word));
        }
        // sshpass -p secret
        if program == "sshpass" && lower == "-p" {
            return Some(keep_quotes(word));
        }
        // curl -u user:secret
        if (lower == "-u" || lower == "--user") && program == "curl" {
            if let Some(colon) = word.find(':') {
                return Some(format!(
                    "{}:{}",
                    &word[..colon],
                    keep_quotes(&word[colon + 1..])
                ));
            }
        }
    }
    // mysql -psecret
    if matches!(program, "mysql" | "mariadb" | "mysqldump" | "mysqladmin")
        && word.starts_with("-p")
        && word.len() > 2
        && !word.starts_with("--")
    {
        return Some(format!("-p{MASK}"));
    }
    if looks_like_token(bare) {
        return Some(keep_quotes(word));
    }
    None
}

/// MASK, with the quotes the original word had, so the line still reads.
fn keep_quotes(word: &str) -> String {
    let lead: String = word
        .chars()
        .take_while(|c| matches!(c, '"' | '\'' | '`'))
        .collect();
    let trail: String = {
        let mut t: Vec<char> = word
            .chars()
            .rev()
            .take_while(|c| matches!(c, '"' | '\'' | '`'))
            .collect();
        t.reverse();
        t.into_iter().collect()
    };
    if lead.len() == word.len() {
        return MASK.to_string();
    }
    format!("{lead}{MASK}{trail}")
}

fn mask_url(word: &str) -> Option<String> {
    let scheme = word.find("://")?;
    let rest_start = scheme + 3;
    let rest = &word[rest_start..];
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let at = authority.rfind('@')?;
    let userinfo = &authority[..at];
    let colon = userinfo.find(':')?;
    if colon + 1 >= userinfo.len() {
        return None;
    }
    Some(format!(
        "{}{}:{MASK}{}",
        &word[..rest_start],
        &userinfo[..colon],
        &word[rest_start + at..]
    ))
}

/// Words shaped like API tokens or keys.
pub fn looks_like_token(w: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "ghp_",
        "gho_",
        "ghu_",
        "ghs_",
        "ghr_",
        "github_pat_",
        "glpat-",
        "xoxb-",
        "xoxp-",
        "xoxa-",
        "xoxs-",
        "xapp-",
        "npm_",
        "pypi-",
        "hf_",
    ];
    if w.len() >= 16 && PREFIXES.iter().any(|p| w.starts_with(p)) {
        return true;
    }
    if w.len() >= 20 && w.starts_with("sk-") {
        return true;
    }
    // AWS access key ids.
    if w.len() == 20
        && (w.starts_with("AKIA") || w.starts_with("ASIA"))
        && w.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return true;
    }
    // JWTs: three base64url parts, the first a JSON object.
    if w.starts_with("eyJ") && w.matches('.').count() == 2 && w.len() > 30 {
        return true;
    }
    // Long random strings with upper case, lower case and digits. Git
    // hashes and UUIDs are lower case, so they stay.
    w.len() >= 32
        && w.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-'))
        && !w.contains('/') // a path
        && w.chars().any(|c| c.is_ascii_digit())
        && w.chars().any(|c| c.is_ascii_lowercase())
        && w.chars().any(|c| c.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(s: &str) -> String {
        redact(s).expect("kept").text
    }

    fn masked(s: &str) -> bool {
        redact(s).expect("kept").masked
    }

    // Token-shaped test values are built at run time, so the repository's
    // pre-commit check for tokens does not trip on them.
    fn fake(prefix: &str, n: usize) -> String {
        format!(
            "{prefix}{}",
            "aB3".repeat(n).chars().take(n).collect::<String>()
        )
    }

    #[test]
    fn plain_commands_stay() {
        for c in [
            "systemctl restart nginx",
            "tail -f /var/log/syslog | grep -i error",
            "git log --oneline -5",
            "docker compose up -d",
            "cd /srv/app && git pull",
            "git show 3f6c2a9d0b6e4f1c8a7d2e5b9c0f1a2b3c4d5e6f",
            "ls /usr/lib/x86_64-linux-gnu/libSomethingVeryLong1234567890.so",
            "export PATH=$PATH:/opt/bin",
        ] {
            let out = redact(c).expect("kept");
            assert_eq!(out.text, c);
            assert!(!out.masked, "{c}");
        }
    }

    #[test]
    fn variables_and_flags_with_secret_names() {
        assert_eq!(
            r("DB_PASSWORD=hunter2 ./migrate"),
            "DB_PASSWORD=[redacted] ./migrate"
        );
        assert_eq!(
            r("export GITHUB_TOKEN=abc"),
            "export GITHUB_TOKEN=[redacted]"
        );
        assert_eq!(
            r("app --password=hunter2 --user bob"),
            "app --password=[redacted] --user bob"
        );
        assert_eq!(r("app --api-key 'k e'"), "app --api-key '[redacted] e'");
        assert_eq!(
            r("vault login -token=s.abc"),
            "vault login -token=[redacted]"
        );
        assert_eq!(
            r("aws configure set aws_secret_access_key=xyz"),
            "aws configure set aws_secret_access_key=[redacted]"
        );
        assert_eq!(
            r("psql --password secret db"),
            "psql --password [redacted] db"
        );
    }

    #[test]
    fn urls_headers_and_tools() {
        assert_eq!(
            r("git clone https://bob:hunter2@git.example.com/repo.git"),
            "git clone https://bob:[redacted]@git.example.com/repo.git"
        );
        assert_eq!(
            r("curl -u bob:hunter2 https://x"),
            "curl -u bob:[redacted] https://x"
        );
        assert_eq!(
            r("curl -H \"Authorization: Bearer abc.def\" https://x"),
            "curl -H \"Authorization: Bearer [redacted]\" https://x"
        );
        assert_eq!(
            r("mysql -u root -phunter2 shop"),
            "mysql -u root -p[redacted] shop"
        );
        assert_eq!(
            r("sshpass -p hunter2 ssh web1"),
            "sshpass -p [redacted] ssh web1"
        );
        // -p is a port or a flag for most programs.
        assert_eq!(r("ssh -p 2222 web1"), "ssh -p 2222 web1");
        assert_eq!(r("mkdir -p /srv/app"), "mkdir -p /srv/app");
    }

    #[test]
    fn token_shapes() {
        let gh = fake("ghp_", 36);
        assert_eq!(
            r(&format!("gh auth login --with-token {gh}")),
            "gh auth login --with-token [redacted]"
        );
        let sk = fake("sk-", 40);
        assert!(masked(&format!("llm ask {sk}")));
        let aws = format!("AKIA{}", "ABCDEFGHIJKLMNOP");
        assert!(masked(&format!("echo {aws}")));
        let jwt = format!(
            "eyJ{}.{}.{}",
            "a".repeat(20),
            "b".repeat(20),
            "c".repeat(10)
        );
        assert!(masked(&format!("curl -d {jwt}")));
        let random = fake("", 40);
        assert!(masked(&format!("echo {random}")));
    }

    #[test]
    fn private_keys_are_dropped() {
        assert!(redact("echo '-----BEGIN OPENSSH PRIVATE KEY-----' > k").is_none());
    }
}
