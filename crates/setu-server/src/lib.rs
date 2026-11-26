//! Server management over SSH: systemd services and their logs, for now.
//!
//! Setu runs a small number of **fixed** commands on the server. The only
//! input that ever reaches them is a unit name, and [`services::check_unit`]
//! lets through only the characters systemd itself allows in unit names
//! (no spaces, quotes or shell characters) before it is quoted anyway.
//! Changes are never typed by the window: it asks for an action on a unit,
//! and Rust builds the command with [`services::action_command`], the same
//! function that built the text the person approved.
//!
//! Everything the server sends back is **untrusted**: it is stripped of
//! control characters and terminal escapes, every field is shortened and
//! every list is capped before it reaches the window.

pub mod services;

/// Removes terminal escape sequences and control characters (tabs and line
/// breaks are kept) from text a server sent.
pub fn plain_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.peek() {
                // CSI: ESC [ … final byte in @..~
                Some('[') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: ESC ] … BEL or ESC \
                Some(']') => {
                    chars.next();
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {
                    chars.next();
                }
            },
            '\n' | '\t' => out.push(c),
            '\r' => {}
            c if c.is_control() => {}
            // Bidirectional overrides could make a line read differently.
            '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' => {}
            c => out.push(c),
        }
    }
    out
}

/// One line of server text for a field: plain, on one line, at most `max`
/// characters.
pub fn field(s: &str, max: usize) -> String {
    let clean = plain_text(s);
    let one_line: String = clean
        .chars()
        .map(|c| if c == '\n' || c == '\t' { ' ' } else { c })
        .collect();
    let t = one_line.trim();
    if t.chars().count() > max {
        let mut cut: String = t.chars().take(max.saturating_sub(1)).collect();
        cut.push('…');
        cut
    } else {
        t.to_string()
    }
}

/// Keeps the last `max` bytes of `s`, cut at a line break when there is
/// one, so a log shows its newest lines whole.
pub fn tail(s: &str, max: usize) -> (String, bool) {
    if s.len() <= max {
        return (s.to_string(), false);
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    let rest = &s[start..];
    let rest = rest.find('\n').map_or(rest, |i| &rest[i + 1..]);
    (rest.to_string(), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_and_controls_are_removed() {
        assert_eq!(plain_text("a\u{1b}[31mred\u{1b}[0m b"), "ared b");
        assert_eq!(plain_text("x\u{1b}]0;title\u{7}y"), "xy");
        assert_eq!(plain_text("x\u{1b}]8;;http://e\u{1b}\\y"), "xy");
        assert_eq!(plain_text("a\r\nb\tc\u{0}\u{7}"), "a\nb\tc");
        assert_eq!(plain_text("ab\u{202e}cd"), "abcd");
    }

    #[test]
    fn fields_are_one_short_line() {
        assert_eq!(field("  a\nb\tc ", 10), "a b c");
        assert_eq!(field("abcdefghij", 5), "abcd…");
        assert_eq!(field("ééééé", 5), "ééééé");
    }

    #[test]
    fn tail_keeps_whole_newest_lines() {
        assert_eq!(
            tail("one\ntwo\nthree\n", 100),
            ("one\ntwo\nthree\n".into(), false)
        );
        assert_eq!(tail("one\ntwo\nthree\n", 9), ("three\n".into(), true));
        // Never splits a character.
        let (t, cut) = tail("ééé\nb", 4);
        assert!(cut);
        assert_eq!(t, "b");
    }
}
