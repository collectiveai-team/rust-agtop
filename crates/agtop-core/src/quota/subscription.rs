//! Subscription-name resolution for quota providers.
//!
//! Each provider has a primary source (the agent-tool's own credential file)
//! and a fallback (opencode's auth.json, which the quota providers already
//! hold via `OpencodeAuth`). The TUI displays the result as `meta["plan"]`.
//!
//! Mirrors the logic in `clients/{claude,codex,copilot,…}.rs` so the quota
//! pane shows the **same subscription label** as the Sessions and Cost panes.

use crate::quota::auth::OpencodeAuth;

// ---------------------------------------------------------------------------
// Claude
// ---------------------------------------------------------------------------

/// Resolve the Claude subscription name.
///
/// Primary: `$CLAUDE_CONFIG_DIR/.credentials.json`
///   → `claudeAiOauth.rateLimitTier` → "Claude Max 5x" / "Claude Max 20x" / "Pro"
///   → fallback to `subscriptionType` (title-cased)
///
/// Fallback (file absent/unreadable): opencode auth.json entry for
///   "anthropic"/"claude" → if oauth → "Claude Max 5x", if API key → "Anthropic API key".
pub fn claude_plan(auth: &OpencodeAuth) -> Option<String> {
    // Primary: Claude Code's own credentials file.
    if let Some(name) = read_claude_credentials() {
        return Some(name);
    }

    // Fallback: opencode auth.json.
    let entry = auth.lookup(&["anthropic", "claude"])?;
    if entry.access.is_some() || entry.token.is_some() {
        // OAuth credential → assume Max (we can't know the exact tier without
        // the credentials file, but this is the same fallback opencode.rs uses).
        Some("Claude Max 5x".to_string())
    } else if entry.key.is_some() {
        Some("Anthropic API key".to_string())
    } else {
        None
    }
}

fn read_claude_credentials() -> Option<String> {
    let home = dirs::home_dir()?;
    let base = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"));
    let path = base.join(".credentials.json");
    let bytes = std::fs::read(&path).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let oauth = v.get("claudeAiOauth")?;

    // Prefer rateLimitTier — canonical human-readable tier string.
    if let Some(tier) = oauth.get("rateLimitTier").and_then(|x| x.as_str()) {
        let mapped = match tier {
            "default_claude_pro" => Some("Pro".to_string()),
            "default_claude_max_5x" => Some("Claude Max 5x".to_string()),
            "default_claude_max_20x" => Some("Claude Max 20x".to_string()),
            _ => None,
        };
        if mapped.is_some() {
            return mapped;
        }
    }

    // Fallback within the file: subscriptionType (title-case first char).
    oauth
        .get("subscriptionType")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
        .map(title_case_first)
}

// ---------------------------------------------------------------------------
// Codex
// ---------------------------------------------------------------------------

/// Resolve the Codex / ChatGPT subscription name.
///
/// Primary: `~/.codex/auth.json` → JWT `id_token` payload
///   → `https://api.openai.com/auth.chatgpt_plan_type`
///   → "ChatGPT Plus" / "ChatGPT Pro" / "ChatGPT Business" / "ChatGPT Enterprise"
///
/// Fallback: opencode auth.json entry for "openai"/"codex"
///   → if oauth → "ChatGPT (OAuth)", if API key → "OpenAI API key".
pub fn codex_plan(auth: &OpencodeAuth) -> Option<String> {
    // Primary: Codex's own auth file.
    if let Some(name) = read_codex_auth() {
        return Some(name);
    }

    // Fallback: opencode auth.json.
    let entry = auth.lookup(&["openai", "codex", "chatgpt"])?;
    if entry.access.is_some() || entry.token.is_some() {
        Some("ChatGPT (OAuth)".to_string())
    } else if entry.key.is_some() {
        Some("OpenAI API key".to_string())
    } else {
        None
    }
}

fn read_codex_auth() -> Option<String> {
    let home = dirs::home_dir()?;
    let path = home.join(".codex").join("auth.json");
    let raw = std::fs::read_to_string(&path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;

    let id_token = v
        .get("tokens")
        .and_then(|t| t.get("id_token"))
        .and_then(|x| x.as_str())?;

    let payload_b64 = id_token.split('.').nth(1)?;
    let bytes = base64url_decode(payload_b64)?;
    let payload: serde_json::Value = serde_json::from_slice(&bytes).ok()?;

    let plan_type = payload
        .get("https://api.openai.com/auth")?
        .get("chatgpt_plan_type")
        .and_then(|x| x.as_str())?;

    Some(map_openai_plan(plan_type))
}

fn map_openai_plan(raw: &str) -> String {
    match raw {
        "plus" => "ChatGPT Plus".to_string(),
        "pro" => "ChatGPT Pro".to_string(),
        "business" => "ChatGPT Business".to_string(),
        "enterprise" => "ChatGPT Enterprise".to_string(),
        other => title_case_first(other),
    }
}

// ---------------------------------------------------------------------------
// Copilot / CopilotAddon
// ---------------------------------------------------------------------------

/// Format the Copilot subscription label from the `copilot_plan` API field.
///
/// The quota API already returns `copilot_plan` ("individual", "business",
/// "enterprise", …) — the same field the session client reads. We format it
/// as "GitHub Copilot · Individual" to match
/// `clients/copilot.rs: format!("GitHub Copilot · {}", p)`.
pub fn copilot_plan(raw_plan: Option<&str>) -> String {
    match raw_plan {
        Some(p) => format!("GitHub Copilot \u{00b7} {}", title_case_first(p)),
        None => "GitHub Copilot".to_string(),
    }
}

/// Same as `copilot_plan` but with the "Add-on" qualifier.
pub fn copilot_addon_plan(raw_plan: Option<&str>) -> String {
    match raw_plan {
        Some(p) => format!("GitHub Copilot Add-on \u{00b7} {}", title_case_first(p)),
        None => "GitHub Copilot Add-on".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Zai
// ---------------------------------------------------------------------------

/// Format the z.ai subscription label from the `level` API field.
///
/// The API returns `level` ("lite", "pro", …). We format it as
/// "z.ai · Lite" / "z.ai · Pro".
pub fn zai_plan(raw_level: Option<&str>) -> String {
    match raw_level {
        Some(l) => format!("z.ai \u{00b7} {}", title_case_first(l)),
        None => "z.ai".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Google
// ---------------------------------------------------------------------------

/// Format the Google subscription label from the `sources` meta field.
///
/// Sources are "Gemini", "Antigravity", or both comma-joined. We format as
/// "Google · Gemini" / "Google · Antigravity" / "Google · Gemini, Antigravity".
pub fn google_plan(raw_sources: Option<&str>) -> String {
    match raw_sources {
        Some(s) if !s.is_empty() => {
            // raw_sources is comma-joined: "Gemini,Antigravity"
            // Display with a space after the comma.
            let formatted = s.replace(',', ", ");
            format!("Google \u{00b7} {formatted}")
        }
        _ => "Google".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn title_case_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// Decode a base64url-encoded string (RFC 4648 §5).
/// Identical to the implementation in `clients/codex.rs`.
fn base64url_decode(input: &str) -> Option<Vec<u8>> {
    let mut s: String = input
        .chars()
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            c => c,
        })
        .collect();
    while s.len() % 4 != 0 {
        s.push('=');
    }

    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for chunk in bytes.chunks(4) {
        if chunk.len() < 4 {
            return None;
        }
        let pad = chunk.iter().rev().take_while(|&&b| b == b'=').count();
        if pad > 2 {
            return None;
        }
        let mut n = 0u32;
        for (i, &b) in chunk.iter().enumerate() {
            let v = if b == b'=' { 0 } else { val(b)? };
            n |= (v as u32) << (18 - 6 * i);
        }
        out.push(((n >> 16) & 0xFF) as u8);
        if pad < 2 {
            out.push(((n >> 8) & 0xFF) as u8);
        }
        if pad < 1 {
            out.push((n & 0xFF) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copilot_plan_formats_individual() {
        assert_eq!(
            copilot_plan(Some("individual")),
            "GitHub Copilot · Individual"
        );
    }

    #[test]
    fn copilot_plan_formats_business() {
        assert_eq!(copilot_plan(Some("business")), "GitHub Copilot · Business");
    }

    #[test]
    fn copilot_plan_none_is_bare_name() {
        assert_eq!(copilot_plan(None), "GitHub Copilot");
    }

    #[test]
    fn copilot_addon_plan_formats_individual() {
        assert_eq!(
            copilot_addon_plan(Some("individual")),
            "GitHub Copilot Add-on · Individual"
        );
    }

    #[test]
    fn zai_plan_formats_lite() {
        assert_eq!(zai_plan(Some("lite")), "z.ai · Lite");
    }

    #[test]
    fn zai_plan_formats_pro() {
        assert_eq!(zai_plan(Some("pro")), "z.ai · Pro");
    }

    #[test]
    fn zai_plan_none_is_bare_name() {
        assert_eq!(zai_plan(None), "z.ai");
    }

    #[test]
    fn google_plan_formats_gemini() {
        assert_eq!(google_plan(Some("Gemini")), "Google · Gemini");
    }

    #[test]
    fn google_plan_formats_both_sources() {
        assert_eq!(
            google_plan(Some("Gemini,Antigravity")),
            "Google · Gemini, Antigravity"
        );
    }

    #[test]
    fn google_plan_none_is_bare_name() {
        assert_eq!(google_plan(None), "Google");
    }

    #[test]
    fn map_openai_plan_known_values() {
        assert_eq!(map_openai_plan("plus"), "ChatGPT Plus");
        assert_eq!(map_openai_plan("pro"), "ChatGPT Pro");
        assert_eq!(map_openai_plan("business"), "ChatGPT Business");
        assert_eq!(map_openai_plan("enterprise"), "ChatGPT Enterprise");
    }

    #[test]
    fn map_openai_plan_unknown_title_cased() {
        assert_eq!(map_openai_plan("team"), "Team");
    }
}
