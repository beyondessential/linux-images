use std::path::Path;

use anyhow::{Context, Result, bail};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

// r[impl installer.dryrun.script]

/// Parse a script file into a sequence of `KeyEvent`s.
///
/// Each line is one of:
/// - A named key: `enter`, `esc`, `tab`, `backspace`, `up`, `down`, `left`,
///   `right`, `space`
/// - `type:<text>` — emits one `Char` keypress per character of `<text>`
/// - `alt:<text>` — emits one `Char` keypress per character with the Alt
///   modifier held (e.g. `alt:t` produces Alt+t)
/// - Lines starting with `#` are comments and are ignored
/// - Empty lines are ignored
pub fn parse_script_file(path: &Path) -> Result<Vec<KeyEvent>> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("reading script {}", path.display()))?;
    parse_script(&contents)
}

pub fn parse_script(contents: &str) -> Result<Vec<KeyEvent>> {
    let mut events = Vec::new();

    for (line_num, raw_line) in contents.lines().enumerate() {
        let line = raw_line.trim();

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some(text) = line.strip_prefix("type:") {
            for ch in text.chars() {
                events.push(make_key(KeyCode::Char(ch)));
            }
            continue;
        }

        if let Some(text) = line.strip_prefix("alt:") {
            for ch in text.chars() {
                events.push(make_alt_key(KeyCode::Char(ch)));
            }
            continue;
        }

        let code = match line {
            "enter" => KeyCode::Enter,
            "esc" => KeyCode::Esc,
            "tab" => KeyCode::Tab,
            "backspace" => KeyCode::Backspace,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "space" => KeyCode::Char(' '),
            other => {
                bail!(
                    "unknown key token '{}' at line {} of input script",
                    other,
                    line_num + 1
                );
            }
        };

        events.push(make_key(code));
    }

    Ok(events)
}

fn make_key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::empty(),
        kind: KeyEventKind::Press,
        state: crossterm::event::KeyEventState::empty(),
    }
}

fn make_alt_key(code: KeyCode) -> KeyEvent {
    KeyEvent {
        code,
        modifiers: KeyModifiers::ALT,
        kind: KeyEventKind::Press,
        state: crossterm::event::KeyEventState::empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // r[verify installer.dryrun.script]
    #[test]
    fn parse_named_keys() {
        let script = "enter\nesc\ntab\nbackspace\nup\ndown\nleft\nright\nspace\n";
        let events = parse_script(script).unwrap();
        assert_eq!(events.len(), 9);
        assert_eq!(events[0].code, KeyCode::Enter);
        assert_eq!(events[1].code, KeyCode::Esc);
        assert_eq!(events[2].code, KeyCode::Tab);
        assert_eq!(events[3].code, KeyCode::Backspace);
        assert_eq!(events[4].code, KeyCode::Up);
        assert_eq!(events[5].code, KeyCode::Down);
        assert_eq!(events[6].code, KeyCode::Left);
        assert_eq!(events[7].code, KeyCode::Right);
        assert_eq!(events[8].code, KeyCode::Char(' '));
    }

    // r[verify installer.dryrun.script]
    #[test]
    fn parse_type_directive() {
        let script = "type:hello";
        let events = parse_script(script).unwrap();
        assert_eq!(events.len(), 5);
        assert_eq!(events[0].code, KeyCode::Char('h'));
        assert_eq!(events[1].code, KeyCode::Char('e'));
        assert_eq!(events[2].code, KeyCode::Char('l'));
        assert_eq!(events[3].code, KeyCode::Char('l'));
        assert_eq!(events[4].code, KeyCode::Char('o'));
    }

    // r[verify installer.dryrun.script]
    #[test]
    fn parse_type_empty_string() {
        let script = "type:";
        let events = parse_script(script).unwrap();
        assert!(events.is_empty());
    }

    // r[verify installer.dryrun.script]
    #[test]
    fn parse_alt_directive() {
        let script = "alt:t";
        let events = parse_script(script).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].code, KeyCode::Char('t'));
        assert!(events[0].modifiers.contains(KeyModifiers::ALT));
    }

    // r[verify installer.dryrun.script]
    #[test]
    fn parse_alt_multiple_chars() {
        let script = "alt:sg";
        let events = parse_script(script).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].code, KeyCode::Char('s'));
        assert!(events[0].modifiers.contains(KeyModifiers::ALT));
        assert_eq!(events[1].code, KeyCode::Char('g'));
        assert!(events[1].modifiers.contains(KeyModifiers::ALT));
    }

    // r[verify installer.dryrun.script]
    #[test]
    fn parse_comments_and_blank_lines() {
        let script = "# this is a comment\n\n  # indented comment\nenter\n\n";
        let events = parse_script(script).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].code, KeyCode::Enter);
    }

    // r[verify installer.dryrun.script]
    #[test]
    fn parse_unknown_token_errors() {
        let script = "enter\nfoobar\n";
        let result = parse_script(script);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("foobar"));
        assert!(err.contains("line 2"));
    }

    // r[verify installer.dryrun.script]
    #[test]
    fn parse_mixed_script() {
        let script = "\
# Welcome screen
enter
# Disk selection — accept default
enter
# DiskEncryption — cycle to None
down
enter
# Hostname
type:my-host
enter
# Tailscale — skip
enter
# SSH keys — skip
tab
# Confirmation
type:yes
enter
";
        let events = parse_script(script).unwrap();
        // enter, enter, down, enter, m,y,-,h,o,s,t, enter, enter, tab, y,e,s, enter
        assert_eq!(events.len(), 18);
    }

    // r[verify installer.dryrun.script]
    #[test]
    fn all_events_are_press_kind() {
        let script = "enter\ntype:ab\nspace\n";
        let events = parse_script(script).unwrap();
        for ev in &events {
            assert_eq!(ev.kind, KeyEventKind::Press);
        }
    }

    // r[verify installer.dryrun.script]
    #[test]
    fn whitespace_trimmed_from_lines() {
        let script = "  enter  \n  type:x  \n";
        let events = parse_script(script).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].code, KeyCode::Enter);
        // Note: type: after trim is "type:x" so we get 'x', but trailing
        // spaces on the raw line are trimmed before prefix matching
        assert_eq!(events[1].code, KeyCode::Char('x'));
    }

    // r[verify installer.dryrun.script]
    #[test]
    fn parse_script_file_nonexistent() {
        let result = parse_script_file(Path::new("/nonexistent/script.txt"));
        assert!(result.is_err());
    }

    // r[verify installer.dryrun.script]
    #[test]
    fn parse_script_file_works() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.script");
        std::fs::write(&path, "enter\ntype:hi\n").unwrap();
        let events = parse_script_file(&path).unwrap();
        assert_eq!(events.len(), 3);
    }
}
