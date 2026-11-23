//! Terminal colour schemes imported from other terminals: iTerm2
//! `.itermcolors`, Windows Terminal JSON, Alacritty TOML, base16/base24 YAML,
//! and the `name value` files of kitty, Ghostty and Xresources.
//!
//! The file is untrusted: only colours are taken from it, each one checked
//! and written back as `#rrggbb`, and the name is cleaned and shortened. Nothing
//! in it is run or stored anywhere but the UI's settings.

use serde::Serialize;
use std::collections::HashMap;

/// Largest file we read; scheme files are a few kilobytes.
const MAX_BYTES: usize = 2 * 1024 * 1024;
/// A Windows Terminal settings file can hold many schemes; keep a sane cap.
const MAX_SCHEMES: usize = 100;
const MAX_NAME: usize = 60;

/// One scheme, every colour as `#rrggbb`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ColorScheme {
    pub name: String,
    pub foreground: String,
    pub background: String,
    pub cursor: Option<String>,
    pub selection: Option<String>,
    /// black, red, green, yellow, blue, magenta, cyan, white, then the bright ones.
    pub ansi: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SchemeScan {
    /// What kind of file it was, for the UI ("iTerm2 colours", …).
    pub format: &'static str,
    pub schemes: Vec<ColorScheme>,
}

#[tauri::command]
pub fn color_scheme_parse(file_name: String, bytes: Vec<u8>) -> Result<SchemeScan, String> {
    parse_file(&file_name, &bytes)
}

pub fn parse_file(file_name: &str, bytes: &[u8]) -> Result<SchemeScan, String> {
    if bytes.len() > MAX_BYTES {
        return Err("that file is over 2 MB; colour schemes are only a few kilobytes".into());
    }
    let stem = file_stem(file_name);
    let lower = file_name.to_ascii_lowercase();
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();

    let (format, schemes) = if lower.ends_with(".itermcolors")
        || trimmed.starts_with("<?xml")
        || bytes.starts_with(b"bplist")
    {
        ("iTerm2 colours", parse_iterm(bytes, &stem)?)
    } else if lower.ends_with(".toml") || trimmed.starts_with("[colors") {
        ("Alacritty colours", parse_alacritty(trimmed, &stem)?)
    } else if trimmed.starts_with('{') || trimmed.starts_with('[') {
        (
            "Windows Terminal scheme",
            parse_windows_terminal(trimmed, &stem)?,
        )
    } else if looks_like_base16(trimmed) {
        ("base16 scheme", parse_base16(trimmed, &stem)?)
    } else {
        ("terminal colours", parse_key_values(trimmed, &stem)?)
    };
    if schemes.is_empty() {
        return Err(unknown());
    }
    Ok(SchemeScan { format, schemes })
}

fn unknown() -> String {
    "found no colour scheme in that file. Setu reads iTerm2 .itermcolors, Windows Terminal JSON, Alacritty .toml, base16 .yaml, and kitty, Ghostty or Xresources colour files".into()
}

fn file_stem(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let stem = match base.rfind('.') {
        Some(i) if i > 0 => &base[..i],
        _ => base,
    };
    clean_name(stem).unwrap_or_else(|| "Imported".into())
}

/// Printable text only, at most [`MAX_NAME`] characters.
fn clean_name(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let cut: String = cleaned.chars().take(MAX_NAME).collect();
    (!cut.is_empty()).then_some(cut)
}

/// Accepts `#rgb`, `#rrggbb`, `rrggbb`, `0xrrggbb`, `#rrggbbaa` and X11's
/// `rgb:r/g/b`; returns `#rrggbb`.
pub fn parse_color(raw: &str) -> Option<String> {
    let s = raw.trim().trim_matches(['"', '\'']).trim();
    if let Some(rest) = s.strip_prefix("rgb:") {
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() != 3 {
            return None;
        }
        let mut out = String::from("#");
        for p in parts {
            if p.is_empty() || p.len() > 4 || !p.chars().all(|c| c.is_ascii_hexdigit()) {
                return None;
            }
            // Scale 1 to 4 hex digits to 8 bits, as XParseColor does.
            let v = u32::from_str_radix(p, 16).ok()?;
            let max = (1u32 << (4 * p.len())) - 1;
            out.push_str(&format!("{:02x}", (v * 255 + max / 2) / max));
        }
        return Some(out);
    }
    let hex = s
        .strip_prefix('#')
        .or_else(|| s.strip_prefix("0x"))
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s);
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let hex = hex.to_ascii_lowercase();
    match hex.len() {
        3 => Some(hex.chars().fold(String::from("#"), |mut o, c| {
            o.push(c);
            o.push(c);
            o
        })),
        6 => Some(format!("#{hex}")),
        8 => Some(format!("#{}", &hex[..6])),
        _ => None,
    }
}

/// Builds a scheme once it has a foreground, a background and all 16 colours.
fn finish(
    name: String,
    fg: Option<String>,
    bg: Option<String>,
    cursor: Option<String>,
    selection: Option<String>,
    ansi: [Option<String>; 16],
) -> Option<ColorScheme> {
    let ansi: Option<Vec<String>> = ansi.into_iter().collect();
    let ansi = ansi?;
    Some(ColorScheme {
        name,
        foreground: fg.unwrap_or_else(|| ansi[7].clone()),
        background: bg.unwrap_or_else(|| ansi[0].clone()),
        cursor,
        selection,
        ansi,
    })
}

// ---- iTerm2 -------------------------------------------------------------

fn parse_iterm(bytes: &[u8], stem: &str) -> Result<Vec<ColorScheme>, String> {
    let value = plist::Value::from_reader(std::io::Cursor::new(bytes))
        .map_err(|_| "that .itermcolors file is not a valid property list".to_string())?;
    let dict = value.as_dictionary().ok_or_else(unknown)?;
    let color = |key: &str| -> Option<String> {
        let d = dict.get(key)?.as_dictionary()?;
        let comp = |k: &str| -> Option<f64> {
            let v = d.get(k)?;
            v.as_real()
                .or_else(|| v.as_signed_integer().map(|i| i as f64))
        };
        let (r, g, b) = (
            comp("Red Component")?,
            comp("Green Component")?,
            comp("Blue Component")?,
        );
        let byte = |c: f64| -> Option<u8> {
            c.is_finite()
                .then(|| (c.clamp(0.0, 1.0) * 255.0).round() as u8)
        };
        Some(format!("#{:02x}{:02x}{:02x}", byte(r)?, byte(g)?, byte(b)?))
    };
    let ansi: [Option<String>; 16] = std::array::from_fn(|i| color(&format!("Ansi {i} Color")));
    Ok(finish(
        stem.to_string(),
        color("Foreground Color"),
        color("Background Color"),
        color("Cursor Color"),
        color("Selection Color"),
        ansi,
    )
    .into_iter()
    .collect())
}

// ---- Windows Terminal -----------------------------------------------------

const WT_NAMES: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "purple",
    "cyan",
    "white",
    "brightBlack",
    "brightRed",
    "brightGreen",
    "brightYellow",
    "brightBlue",
    "brightPurple",
    "brightCyan",
    "brightWhite",
];

fn parse_windows_terminal(text: &str, stem: &str) -> Result<Vec<ColorScheme>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|_| "that JSON file could not be read".to_string())?;
    // A single scheme, a list of them, or a whole settings.json with "schemes".
    let list: Vec<&serde_json::Value> = match &value {
        serde_json::Value::Array(a) => a.iter().collect(),
        serde_json::Value::Object(o) => match o.get("schemes") {
            Some(serde_json::Value::Array(a)) => a.iter().collect(),
            _ => vec![&value],
        },
        _ => vec![],
    };
    Ok(list
        .into_iter()
        .take(MAX_SCHEMES)
        .filter_map(|v| {
            let o = v.as_object()?;
            let get = |k: &str| o.get(k).and_then(|v| v.as_str()).and_then(parse_color);
            // Accept "magenta" too, which some generators write.
            let ansi: [Option<String>; 16] = std::array::from_fn(|i| {
                get(WT_NAMES[i]).or_else(|| {
                    get(&WT_NAMES[i]
                        .replace("purple", "magenta")
                        .replace("Purple", "Magenta"))
                })
            });
            let name = o
                .get("name")
                .and_then(|v| v.as_str())
                .and_then(clean_name)
                .unwrap_or_else(|| stem.to_string());
            finish(
                name,
                get("foreground"),
                get("background"),
                get("cursorColor"),
                get("selectionBackground"),
                ansi,
            )
        })
        .collect())
}

// ---- Alacritty ------------------------------------------------------------

const ANSI_NAMES: [&str; 8] = [
    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
];

fn parse_alacritty(text: &str, stem: &str) -> Result<Vec<ColorScheme>, String> {
    let value: toml::Table = text
        .parse()
        .map_err(|_| "that TOML file could not be read".to_string())?;
    let colors = value
        .get("colors")
        .and_then(|v| v.as_table())
        .ok_or_else(unknown)?;
    let get = |section: &str, key: &str| -> Option<String> {
        colors
            .get(section)?
            .as_table()?
            .get(key)?
            .as_str()
            .and_then(parse_color)
    };
    let ansi: [Option<String>; 16] =
        std::array::from_fn(|i| get(if i < 8 { "normal" } else { "bright" }, ANSI_NAMES[i % 8]));
    Ok(finish(
        stem.to_string(),
        get("primary", "foreground"),
        get("primary", "background"),
        get("cursor", "cursor"),
        get("selection", "background"),
        ansi,
    )
    .into_iter()
    .collect())
}

// ---- base16 / base24 ------------------------------------------------------

/// Simple `key: value` lines (base16 YAML has no nesting worth reading).
fn yaml_pairs(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let l = l.split(" #").next().unwrap_or(l).trim();
            let (k, v) = l.split_once(':')?;
            Some((
                k.trim().trim_matches('"').to_ascii_lowercase(),
                v.trim().trim_matches(['"', '\'']).to_string(),
            ))
        })
        .collect()
}

fn looks_like_base16(text: &str) -> bool {
    let pairs = yaml_pairs(text);
    pairs.contains_key("base00") && pairs.contains_key("base0f")
}

fn parse_base16(text: &str, stem: &str) -> Result<Vec<ColorScheme>, String> {
    let pairs = yaml_pairs(text);
    let b = |k: &str| pairs.get(k).and_then(|v| parse_color(v));
    // The mapping base16-shell uses; base24 adds its own bright colours.
    let normal = [
        "base00", "base08", "base0b", "base0a", "base0d", "base0e", "base0c", "base05",
    ];
    let bright = if pairs.contains_key("base12") {
        [
            "base02", "base12", "base14", "base13", "base16", "base17", "base15", "base07",
        ]
    } else {
        [
            "base03", "base08", "base0b", "base0a", "base0d", "base0e", "base0c", "base07",
        ]
    };
    let ansi: [Option<String>; 16] =
        std::array::from_fn(|i| b(if i < 8 { normal[i] } else { bright[i - 8] }));
    let name = pairs
        .get("scheme")
        .or_else(|| pairs.get("name"))
        .and_then(|n| clean_name(n))
        .unwrap_or_else(|| stem.to_string());
    Ok(finish(
        name,
        b("base05"),
        b("base00"),
        b("base05"),
        b("base02"),
        ansi,
    )
    .into_iter()
    .collect())
}

// ---- kitty, Ghostty, Xresources -------------------------------------------

fn parse_key_values(text: &str, stem: &str) -> Result<Vec<ColorScheme>, String> {
    let mut ansi: [Option<String>; 16] = Default::default();
    let (mut fg, mut bg, mut cursor, mut selection) = (None, None, None, None);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(['#', '!', ';']) {
            continue;
        }
        // `key value`, `key = value` or `key: value`.
        let (key, value) = match line.find(['=', ':', ' ', '\t']) {
            Some(i) => {
                let (k, v) = line.split_at(i);
                (
                    k.trim(),
                    v[1..].trim().trim_start_matches(['=', ':']).trim(),
                )
            }
            None => continue,
        };
        // Xresources keys look like `*.color4` or `URxvt*background`.
        let key = key
            .rsplit(['*', '.'])
            .next()
            .unwrap_or(key)
            .to_ascii_lowercase();
        if key == "palette" {
            // Ghostty: `palette = 4=#0000ff`.
            if let Some((n, c)) = value.split_once('=') {
                if let (Ok(n), Some(c)) = (n.trim().parse::<usize>(), parse_color(c)) {
                    if n < 16 {
                        ansi[n] = Some(c);
                    }
                }
            }
            continue;
        }
        let Some(color) = parse_color(value) else {
            continue;
        };
        if let Some(n) = key
            .strip_prefix("color")
            .and_then(|n| n.parse::<usize>().ok())
        {
            if n < 16 {
                ansi[n] = Some(color);
            }
            continue;
        }
        match key.as_str() {
            "foreground" => fg = Some(color),
            "background" => bg = Some(color),
            "cursor" | "cursorcolor" | "cursor-color" => cursor = Some(color),
            "selection_background" | "selection-background" => selection = Some(color),
            _ => {}
        }
    }
    Ok(finish(stem.to_string(), fg, bg, cursor, selection, ansi)
        .into_iter()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_in_every_spelling() {
        assert_eq!(parse_color("#ABC").as_deref(), Some("#aabbcc"));
        assert_eq!(parse_color("#1e1E2e").as_deref(), Some("#1e1e2e"));
        assert_eq!(parse_color("0x282c34").as_deref(), Some("#282c34"));
        assert_eq!(parse_color("'282c34'").as_deref(), Some("#282c34"));
        assert_eq!(parse_color("#282c34ff").as_deref(), Some("#282c34"));
        assert_eq!(parse_color("rgb:ff/80/00").as_deref(), Some("#ff8000"));
        assert_eq!(
            parse_color("rgb:ffff/0000/8080").as_deref(),
            Some("#ff0080")
        );
        assert_eq!(parse_color("red"), None);
        assert_eq!(parse_color("#12345"), None);
        assert_eq!(parse_color("url(javascript:x)"), None);
        assert_eq!(parse_color("#ff0000;background:url(x)"), None);
    }

    fn iterm_color(name: &str, r: f64, g: f64, b: f64) -> String {
        format!("<key>{name}</key><dict><key>Blue Component</key><real>{b}</real><key>Green Component</key><real>{g}</real><key>Red Component</key><real>{r}</real></dict>")
    }

    #[test]
    fn reads_itermcolors() {
        let mut body = String::new();
        for i in 0..16 {
            body.push_str(&iterm_color(
                &format!("Ansi {i} Color"),
                i as f64 / 15.0,
                0.0,
                1.0,
            ));
        }
        body.push_str(&iterm_color("Foreground Color", 1.0, 1.0, 1.0));
        body.push_str(&iterm_color("Background Color", 0.0, 0.0, 0.0));
        body.push_str(&iterm_color("Cursor Color", 1.0, 0.5, 0.0));
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict>{body}</dict></plist>"#
        );
        let scan = parse_file("/x/Dracula.itermcolors", xml.as_bytes()).expect("parses");
        assert_eq!(scan.format, "iTerm2 colours");
        let s = &scan.schemes[0];
        assert_eq!(s.name, "Dracula");
        assert_eq!(s.ansi[0], "#0000ff");
        assert_eq!(s.ansi[15], "#ff00ff");
        assert_eq!(s.foreground, "#ffffff");
        assert_eq!(s.cursor.as_deref(), Some("#ff8000"));
        assert_eq!(s.selection, None);
    }

    #[test]
    fn itermcolors_missing_a_colour_is_refused() {
        let xml = format!(
            r#"<?xml version="1.0"?><plist version="1.0"><dict>{}</dict></plist>"#,
            iterm_color("Ansi 0 Color", 0.0, 0.0, 0.0)
        );
        assert!(parse_file("a.itermcolors", xml.as_bytes()).is_err());
    }

    const WT: &str = r##"{
        "name": "One Half Dark", "background": "#282C34", "foreground": "#DCDFE4",
        "cursorColor": "#A3B3CC", "selectionBackground": "#474E5D",
        "black": "#282C34", "red": "#E06C75", "green": "#98C379", "yellow": "#E5C07B",
        "blue": "#61AFEF", "purple": "#C678DD", "cyan": "#56B6C2", "white": "#DCDFE4",
        "brightBlack": "#5A6374", "brightRed": "#E06C75", "brightGreen": "#98C379", "brightYellow": "#E5C07B",
        "brightBlue": "#61AFEF", "brightPurple": "#C678DD", "brightCyan": "#56B6C2", "brightWhite": "#DCDFE4"
    }"##;

    #[test]
    fn reads_windows_terminal_scheme_and_settings() {
        let scan = parse_file("scheme.json", WT.as_bytes()).expect("parses");
        let s = &scan.schemes[0];
        assert_eq!(s.name, "One Half Dark");
        assert_eq!(s.background, "#282c34");
        assert_eq!(s.ansi[5], "#c678dd");
        assert_eq!(s.selection.as_deref(), Some("#474e5d"));

        let settings = format!(r#"{{"profiles": {{}}, "schemes": [{WT}, {{"name": "broken"}}]}}"#);
        let scan = parse_file("settings.json", settings.as_bytes()).expect("parses");
        assert_eq!(scan.schemes.len(), 1, "incomplete schemes are skipped");
    }

    #[test]
    fn reads_alacritty_toml() {
        let toml = r##"
[colors.primary]
background = "#1e1e2e"
foreground = "#cdd6f4"
[colors.cursor]
cursor = "#f5e0dc"
[colors.normal]
black = "#45475a"
red = "#f38ba8"
green = "#a6e3a1"
yellow = "#f9e2af"
blue = "#89b4fa"
magenta = "#f5c2e7"
cyan = "#94e2d5"
white = "#bac2de"
[colors.bright]
black = "0x585b70"
red = "#f38ba8"
green = "#a6e3a1"
yellow = "#f9e2af"
blue = "#89b4fa"
magenta = "#f5c2e7"
cyan = "#94e2d5"
white = "#a6adc8"
"##;
        let scan = parse_file("catppuccin-mocha.toml", toml.as_bytes()).expect("parses");
        let s = &scan.schemes[0];
        assert_eq!(s.name, "catppuccin-mocha");
        assert_eq!(s.ansi[8], "#585b70");
        assert_eq!(s.cursor.as_deref(), Some("#f5e0dc"));
    }

    #[test]
    fn reads_base16_yaml() {
        let yaml = "scheme: \"Tomorrow Night\"\nauthor: \"x\"\nbase00: \"1d1f21\"\nbase01: \"282a2e\"\nbase02: \"373b41\"\nbase03: \"969896\"\nbase04: \"b4b7b4\"\nbase05: \"c5c8c6\"\nbase06: \"e0e0e0\"\nbase07: \"ffffff\"\nbase08: \"cc6666\"\nbase09: \"de935f\"\nbase0A: \"f0c674\"\nbase0B: \"b5bd68\"\nbase0C: \"8abeb7\"\nbase0D: \"81a2be\"\nbase0E: \"b294bb\"\nbase0F: \"a3685a\"\n";
        let scan = parse_file("tomorrow-night.yaml", yaml.as_bytes()).expect("parses");
        assert_eq!(scan.format, "base16 scheme");
        let s = &scan.schemes[0];
        assert_eq!(s.name, "Tomorrow Night");
        assert_eq!(s.background, "#1d1f21");
        assert_eq!(s.ansi[1], "#cc6666");
        assert_eq!(s.ansi[8], "#969896");
        assert_eq!(s.ansi[15], "#ffffff");
    }

    fn sixteen(prefix: &str, sep: &str) -> String {
        (0..16)
            .map(|i| format!("{prefix}{i}{sep}#{:02x}0000\n", i * 16))
            .collect()
    }

    #[test]
    fn reads_kitty_ghostty_and_xresources() {
        let kitty = format!(
            "# kitty\nforeground #ffffff\nbackground #000000\nselection_background #333333\n{}",
            sixteen("color", " ")
        );
        let s = &parse_file("theme.conf", kitty.as_bytes())
            .expect("kitty")
            .schemes[0];
        assert_eq!(s.ansi[2], "#200000");
        assert_eq!(s.selection.as_deref(), Some("#333333"));

        let ghostty: String = (0..16)
            .map(|i| format!("palette = {i}=#00{:02x}00\n", i * 16))
            .collect::<String>()
            + "background = 101010\nforeground = eeeeee\ncursor-color = ff0000\n";
        let s = &parse_file("ghostty-theme", ghostty.as_bytes())
            .expect("ghostty")
            .schemes[0];
        assert_eq!(s.ansi[1], "#001000");
        assert_eq!(s.background, "#101010");
        assert_eq!(s.cursor.as_deref(), Some("#ff0000"));

        let xres = format!(
            "! Xresources\n*.foreground: #ffffff\n*.background: rgb:00/00/00\n{}",
            sixteen("*.color", ": ")
        );
        let s = &parse_file(".Xresources", xres.as_bytes())
            .expect("xresources")
            .schemes[0];
        assert_eq!(s.ansi[15], "#f00000");
        assert_eq!(s.background, "#000000");
    }

    #[test]
    fn names_are_cleaned_and_untrusted_input_refused() {
        let wt = WT.replace("One Half Dark", "Evil\\u001b]0;pwned\\u0007 name that goes on and on and on and on and on and on forever");
        let s = &parse_file("x.json", wt.as_bytes()).expect("parses").schemes[0];
        assert!(!s.name.chars().any(char::is_control));
        assert!(s.name.chars().count() <= MAX_NAME);

        assert!(parse_file("x.txt", b"hello world").is_err());
        assert!(parse_file("x.json", b"{not json").is_err());
        assert!(parse_file("x.json", &vec![b' '; MAX_BYTES + 1]).is_err());
    }
}
