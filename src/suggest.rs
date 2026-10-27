//! Inline command suggestions for the terminal: what to offer after the characters typed so far.
//!
//! Candidates come from, in order: this host's history, all history, snippets, then a built-in
//! list of common commands. The first candidate that starts with the typed text wins.

use std::sync::OnceLock;

use regex::Regex;

use crate::model::{HistoryEntry, Snippet};

/// Shortest typed text that gets a suggestion; one letter would flicker on every keystroke.
const MIN_CHARS: usize = 2;
const BUILTIN: &str = include_str!("suggest_commands.txt");

fn builtin() -> &'static [&'static str] {
    static COMMANDS: OnceLock<Vec<&'static str>> = OnceLock::new();
    COMMANDS.get_or_init(|| {
        BUILTIN
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .collect()
    })
}

/// The text to show after the cursor for `typed`, or `None` if nothing fits.
pub fn suggest(
    typed: &str,
    host: &str,
    history: &[HistoryEntry],
    snippets: &[Snippet],
) -> Option<String> {
    if typed.chars().count() < MIN_CHARS {
        return None;
    }
    let fits = |c: &&str| c.len() > typed.len() && c.starts_with(typed) && !c.contains('\n');
    let recent = || history.iter().rev();
    recent()
        .filter(|h| h.host_label == host)
        .map(|h| h.command.as_str())
        .find(fits)
        .or_else(|| recent().map(|h| h.command.as_str()).find(fits))
        // A snippet with {{variables}} would be typed out with the placeholders in it.
        .or_else(|| {
            snippets
                .iter()
                .map(|s| s.script.as_str())
                .filter(|s| !s.contains("{{"))
                .find(fits)
        })
        .or_else(|| builtin().iter().copied().find(fits))
        .map(|c| c[typed.len()..].to_string())
}

/// Whether the screen text before the cursor is a prompt for a password or similar secret.
/// Such input is not echoed, so it must never reach the history or be offered back.
pub fn is_secret_prompt(before_cursor: &str) -> bool {
    static PROMPT: OnceLock<Regex> = OnceLock::new();
    let re = PROMPT.get_or_init(|| {
        Regex::new(
            r"(?i)(password|passphrase|passcode|\bpin\b|\bsecret\b|\btoken\b|\botp\b|verification code|2fa)[^:]*:\s*\**\s*$",
        )
        .expect("secret prompt pattern")
    });
    let tail = before_cursor
        .char_indices()
        .rev()
        .nth(199)
        .map_or(before_cursor, |(i, _)| &before_cursor[i..]);
    re.is_match(tail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::now_ts;
    use uuid::Uuid;

    fn entry(command: &str, host: &str) -> HistoryEntry {
        HistoryEntry {
            id: Uuid::new_v4(),
            command: command.into(),
            host_label: host.into(),
            timestamp: now_ts(),
        }
    }

    fn snippet(script: &str) -> Snippet {
        Snippet {
            script: script.into(),
            ..Default::default()
        }
    }

    #[test]
    fn newest_history_entry_wins() {
        let history = [entry("git status", "a"), entry("git stash", "a")];
        assert_eq!(
            suggest("git st", "a", &history, &[]).as_deref(),
            Some("ash")
        );
    }

    #[test]
    fn same_host_history_beats_newer_history_from_another_host() {
        let history = [entry("ls -la /srv", "web"), entry("ls -lh /tmp", "db")];
        assert_eq!(
            suggest("ls -l", "web", &history, &[]).as_deref(),
            Some("a /srv")
        );
        assert_eq!(
            suggest("ls -l", "other", &history, &[]).as_deref(),
            Some("h /tmp")
        );
    }

    #[test]
    fn history_beats_snippets_and_snippets_beat_builtins() {
        let snippets = [snippet("docker ps --format json")];
        assert_eq!(
            suggest("docker ps", "a", &[], &snippets).as_deref(),
            Some(" --format json")
        );
        let history = [entry("docker ps -q", "a")];
        assert_eq!(
            suggest("docker ps", "a", &history, &snippets).as_deref(),
            Some(" -q")
        );
        assert_eq!(suggest("docker ps", "a", &[], &[]).as_deref(), Some(" -a"));
    }

    #[test]
    fn falls_back_to_common_commands() {
        assert_eq!(suggest("git sta", "a", &[], &[]).as_deref(), Some("tus"));
        assert_eq!(
            suggest("docker compose up", "a", &[], &[]).as_deref(),
            Some(" -d")
        );
        assert_eq!(
            suggest("systemctl res", "a", &[], &[]).as_deref(),
            Some("tart ")
        );
        assert_eq!(
            suggest("sudo systemctl res", "a", &[], &[]).as_deref(),
            Some("tart ")
        );
    }

    #[test]
    fn snippets_with_placeholders_are_never_offered() {
        let snippets = [snippet("tail -n {{lines}} -f {{file}}")];
        assert_eq!(suggest("tail -n {", "a", &[], &snippets), None);
        assert_eq!(suggest("tail -n {{", "a", &[], &snippets), None);
    }

    #[test]
    fn nothing_is_offered_for_short_exact_or_multiline_input() {
        let history = [entry("zzqq", "a"), entry("echo one\necho two", "a")];
        assert_eq!(suggest("l", "a", &history, &[]), None);
        assert_eq!(suggest("zzqq", "a", &history, &[]), None);
        assert_eq!(suggest("echo one", "a", &history, &[]), None);
    }

    #[test]
    fn matching_is_case_sensitive_and_handles_multibyte_text() {
        let history = [entry("echo héllo wörld", "a")];
        assert_eq!(
            suggest("echo hé", "a", &history, &[]).as_deref(),
            Some("llo wörld")
        );
        assert_eq!(suggest("ECHO hé", "a", &history, &[]), None);
    }

    #[test]
    fn builtin_list_is_clean() {
        let commands = builtin();
        assert!(commands.len() > 300);
        for c in commands {
            assert!(!c.contains('\t') && !c.contains('\r'), "{c:?}");
            assert_eq!(c.trim_start(), *c, "{c:?} starts with whitespace");
        }
        let unique: std::collections::HashSet<_> = commands.iter().collect();
        assert_eq!(unique.len(), commands.len(), "duplicate built-in command");
    }

    #[test]
    fn detects_secret_prompts() {
        for prompt in [
            "[sudo] password for fikri: ",
            "Password: ",
            "Enter passphrase for key '/home/u/.ssh/id_ed25519': ",
            "Verification code: ",
            "user@host's password: ****",
            "Enter your PIN: ",
            "API token: ",
        ] {
            assert!(is_secret_prompt(prompt), "{prompt:?}");
        }
        for line in [
            "user@host:~$ ls",
            "user@host:~$ ping 1.1.1.1",
            "user@host:~/passwords$ ",
            "user@host:~$ echo token",
            "# ",
        ] {
            assert!(!is_secret_prompt(line), "{line:?}");
        }
    }
}
