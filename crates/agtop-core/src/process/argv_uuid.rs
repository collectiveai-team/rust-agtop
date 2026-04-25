//! argv-tier session UUID extractor.
//!
//! Inspects the argv of a candidate process and recognizes the
//! per-CLI session-resume invocation patterns we care about. Returns
//! the captured session UUID when one is present, `None` otherwise.
//!
//! Used by the argv tier of the PID correlator (see
//! `docs/specs/2026-04-24-session-pid-tracking-design.md`).

use crate::session::ClientKind;

/// Validate a session UUID in canonical 8-4-4-4-12 lowercase-hex form.
///
/// We deliberately reject mixed/upper case, non-hex characters, and any
/// length deviation. A hand-rolled validator is used in lieu of a
/// `regex` dependency.
///
/// We accept only canonical lowercase form. Both Codex and OpenCode emit
/// lowercase UUIDs in their argv invocations; rejecting uppercase here
/// surfaces upstream format changes loudly rather than silently.
pub(crate) fn is_valid_uuid(s: &str) -> bool {
    // 8 + 1 + 4 + 1 + 4 + 1 + 4 + 1 + 12 == 36
    if s.len() != 36 {
        return false;
    }
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match i {
            8 | 13 | 18 | 23 => {
                if b != b'-' {
                    return false;
                }
            }
            _ => {
                if !matches!(b, b'0'..=b'9' | b'a'..=b'f') {
                    return false;
                }
            }
        }
    }
    true
}

/// Return `Some(value)` when `tok` is `flag=value`. Useful for matching
/// the `--resume=<uuid>` / `--session=<uuid>` shapes. Allocation-free.
fn eq_value<'a>(tok: &'a str, flag: &str) -> Option<&'a str> {
    tok.strip_prefix(flag)
        .and_then(|rest| rest.strip_prefix('='))
}

/// Scan tokens for a per-flag UUID value. Supports both the separated
/// (`--flag <value>`) and `=`-joined (`--flag=value`) shapes. `flags`
/// is the set of accepted flag names (e.g. `["-r", "--resume"]`).
fn find_flag_uuid(tokens: &[String], flags: &[&str]) -> Option<String> {
    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i].as_str();
        for &flag in flags {
            if tok == flag {
                if let Some(next) = tokens.get(i + 1) {
                    if is_valid_uuid(next) {
                        return Some(next.clone());
                    }
                }
            } else if let Some(val) = eq_value(tok, flag) {
                if is_valid_uuid(val) {
                    return Some(val.to_string());
                }
            }
        }
        i += 1;
    }
    None
}

/// Scan tokens for a positional subcommand keyword followed by a UUID
/// (`<keyword> <uuid>`). Used for Codex's `resume <uuid>` and
/// `fork <uuid>` forms. Intervening flags are tolerated automatically:
/// we only inspect the token immediately following each keyword
/// occurrence, but if it's not a UUID we just keep scanning.
fn find_subcommand_uuid(tokens: &[String], keywords: &[&str]) -> Option<String> {
    for (i, tok) in tokens.iter().enumerate() {
        if keywords.contains(&tok.as_str()) {
            if let Some(next) = tokens.get(i + 1) {
                if is_valid_uuid(next) {
                    return Some(next.clone());
                }
            }
        }
    }
    None
}

/// Extract the session UUID a process was launched to resume, if any.
///
/// Skips `argv[0]` (the binary path). For any non-resuming invocation
/// or any unsupported `ClientKind`, returns `None`.
#[allow(dead_code)]
pub(crate) fn extract_session_uuid(client: ClientKind, argv: &[String]) -> Option<String> {
    if argv.len() < 2 {
        return None;
    }
    let tokens = &argv[1..];
    match client {
        // Claude has TWO flags that name a session UUID in argv:
        //   * `-r/--resume <uuid>`  — resume a specific past session
        //   * `--session-id <uuid>` — start a new session with this ID
        // Both bind a process to a session for our purposes.
        ClientKind::Claude => find_flag_uuid(tokens, &["-r", "--resume", "--session-id"]),
        ClientKind::GeminiCli => find_flag_uuid(tokens, &["-r", "--resume"]),
        ClientKind::Codex => find_subcommand_uuid(tokens, &["resume", "fork"]),
        ClientKind::OpenCode => find_flag_uuid(tokens, &["-s", "--session"]),
        ClientKind::Copilot | ClientKind::Cursor | ClientKind::Antigravity => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "12345678-1234-1234-1234-123456789abc";

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| (*s).to_string()).collect()
    }

    // ---- Claude ----------------------------------------------------------

    #[test]
    fn claude_long_resume_with_uuid() {
        let a = argv(&["claude", "--resume", UUID]);
        assert_eq!(
            extract_session_uuid(ClientKind::Claude, &a).as_deref(),
            Some(UUID)
        );
    }

    #[test]
    fn claude_short_resume_with_uuid() {
        let a = argv(&["claude", "-r", UUID]);
        assert_eq!(
            extract_session_uuid(ClientKind::Claude, &a).as_deref(),
            Some(UUID)
        );
    }

    #[test]
    fn claude_eq_form_resume() {
        let a = argv(&["claude", &format!("--resume={UUID}")]);
        assert_eq!(
            extract_session_uuid(ClientKind::Claude, &a).as_deref(),
            Some(UUID)
        );
    }

    #[test]
    fn claude_resume_latest_is_none() {
        let a = argv(&["claude", "--resume", "latest"]);
        assert_eq!(extract_session_uuid(ClientKind::Claude, &a), None);
    }

    #[test]
    fn claude_resume_search_term_is_none() {
        let a = argv(&["claude", "--resume", "fix-the-bug"]);
        assert_eq!(extract_session_uuid(ClientKind::Claude, &a), None);
    }

    #[test]
    fn claude_session_id_with_uuid() {
        // `claude --session-id <uuid>` starts a new session with the
        // given ID. Argv-tier must extract it.
        let a = argv(&["claude", "--session-id", UUID]);
        assert_eq!(
            extract_session_uuid(ClientKind::Claude, &a).as_deref(),
            Some(UUID)
        );
    }

    #[test]
    fn claude_session_id_eq_form() {
        let a = argv(&["claude", &format!("--session-id={UUID}")]);
        assert_eq!(
            extract_session_uuid(ClientKind::Claude, &a).as_deref(),
            Some(UUID)
        );
    }

    // ---- Codex -----------------------------------------------------------

    #[test]
    fn codex_resume_positional_with_uuid() {
        let a = argv(&["codex", "resume", UUID]);
        assert_eq!(
            extract_session_uuid(ClientKind::Codex, &a).as_deref(),
            Some(UUID)
        );
    }

    #[test]
    fn codex_resume_with_intervening_global_flag() {
        let a = argv(&["codex", "-c", "model=opus", "resume", UUID]);
        assert_eq!(
            extract_session_uuid(ClientKind::Codex, &a).as_deref(),
            Some(UUID)
        );
    }

    #[test]
    fn codex_fork_with_uuid() {
        let a = argv(&["codex", "fork", UUID]);
        assert_eq!(
            extract_session_uuid(ClientKind::Codex, &a).as_deref(),
            Some(UUID)
        );
    }

    #[test]
    fn codex_no_resume_no_fork_is_none() {
        let a = argv(&["codex", "exec", "do something"]);
        assert_eq!(extract_session_uuid(ClientKind::Codex, &a), None);
    }

    // ---- Gemini ----------------------------------------------------------

    #[test]
    fn gemini_long_resume_with_uuid() {
        let a = argv(&["node", "/opt/gemini/bin/gemini", "--resume", UUID]);
        assert_eq!(
            extract_session_uuid(ClientKind::GeminiCli, &a).as_deref(),
            Some(UUID)
        );
    }

    #[test]
    fn gemini_resume_latest_is_none() {
        let a = argv(&["node", "/opt/gemini/bin/gemini", "--resume", "latest"]);
        assert_eq!(extract_session_uuid(ClientKind::GeminiCli, &a), None);
    }

    #[test]
    fn gemini_resume_numeric_index_is_none() {
        let a = argv(&["node", "/opt/gemini/bin/gemini", "--resume", "5"]);
        assert_eq!(extract_session_uuid(ClientKind::GeminiCli, &a), None);
    }

    // ---- OpenCode --------------------------------------------------------

    #[test]
    fn opencode_run_short_session() {
        let a = argv(&["opencode", "run", "-s", UUID]);
        assert_eq!(
            extract_session_uuid(ClientKind::OpenCode, &a).as_deref(),
            Some(UUID)
        );
    }

    #[test]
    fn opencode_run_long_session() {
        let a = argv(&["opencode", "run", "--session", UUID]);
        assert_eq!(
            extract_session_uuid(ClientKind::OpenCode, &a).as_deref(),
            Some(UUID)
        );
    }

    #[test]
    fn opencode_run_eq_session() {
        let a = argv(&["opencode", "run", &format!("--session={UUID}")]);
        assert_eq!(
            extract_session_uuid(ClientKind::OpenCode, &a).as_deref(),
            Some(UUID)
        );
    }

    // ---- Unsupported clients --------------------------------------------

    #[test]
    fn copilot_always_none() {
        let a = argv(&["copilot", "--resume", UUID]);
        assert_eq!(extract_session_uuid(ClientKind::Copilot, &a), None);
    }

    #[test]
    fn cursor_always_none() {
        let a = argv(&["cursor", "-r", UUID]);
        assert_eq!(extract_session_uuid(ClientKind::Cursor, &a), None);
    }

    #[test]
    fn antigravity_always_none() {
        let a = argv(&["antigravity", "--resume", UUID]);
        assert_eq!(extract_session_uuid(ClientKind::Antigravity, &a), None);
    }

    // ---- Edge cases ------------------------------------------------------

    #[test]
    fn empty_argv_is_none() {
        let a: Vec<String> = vec![];
        assert_eq!(extract_session_uuid(ClientKind::Claude, &a), None);
    }

    #[test]
    fn argv0_only_is_none() {
        let a = argv(&["claude"]);
        assert_eq!(extract_session_uuid(ClientKind::Claude, &a), None);
    }

    #[test]
    fn claude_does_not_match_opencode_session_flag() {
        // -s belongs to OpenCode; under Claude it must not match.
        let a = argv(&["claude", "-s", UUID]);
        assert_eq!(extract_session_uuid(ClientKind::Claude, &a), None);
    }

    #[test]
    fn opencode_does_not_match_resume_flag() {
        // --resume belongs to Claude/Gemini; under OpenCode it must not match.
        let a = argv(&["opencode", "run", "--resume", UUID]);
        assert_eq!(extract_session_uuid(ClientKind::OpenCode, &a), None);
    }

    #[test]
    fn mixed_case_uuid_is_rejected() {
        let upper = "12345678-1234-1234-1234-123456789ABC";
        let a = argv(&["claude", "--resume", upper]);
        assert_eq!(extract_session_uuid(ClientKind::Claude, &a), None);
    }

    #[test]
    fn wrong_length_uuid_is_rejected() {
        // 35 chars (one short)
        let short = "12345678-1234-1234-1234-123456789ab";
        let a = argv(&["claude", "--resume", short]);
        assert_eq!(extract_session_uuid(ClientKind::Claude, &a), None);

        // 37 chars (one too many)
        let long = "12345678-1234-1234-1234-123456789abcd";
        let a = argv(&["claude", "--resume", long]);
        assert_eq!(extract_session_uuid(ClientKind::Claude, &a), None);
    }

    #[test]
    fn non_hex_in_uuid_is_rejected() {
        let bad = "12345678-1234-1234-1234-12345678zzzz";
        let a = argv(&["claude", "--resume", bad]);
        assert_eq!(extract_session_uuid(ClientKind::Claude, &a), None);
    }

    #[test]
    fn missing_hyphens_in_uuid_is_rejected() {
        let bad = "123456781234123412341234123456789abc"; // 36 chars, no hyphens
        let a = argv(&["claude", "--resume", bad]);
        assert_eq!(extract_session_uuid(ClientKind::Claude, &a), None);
    }

    #[test]
    fn is_valid_uuid_basic() {
        assert!(is_valid_uuid(UUID));
        assert!(!is_valid_uuid(""));
        assert!(!is_valid_uuid("not-a-uuid"));
    }
}
