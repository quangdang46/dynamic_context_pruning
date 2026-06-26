//! `token-stats` subcommand — aggregate token usage across sessions.

use clap::Parser;
use std::collections::BTreeMap;

#[cfg(feature = "scripts")]
use crate::commands::db::OpencodeDb;

/// Aggregate token usage statistics across sessions.
///
/// By default, shows per-session breakdown and grand totals across the N most
/// recent sessions (default 10). Use `--session` to focus on a single session.
#[derive(Parser, Debug)]
pub struct Args {
    /// Number of recent sessions to aggregate across.
    #[arg(long = "sessions", short = 'n', default_value = "10")]
    pub sessions: usize,

    /// Focus on a single session instead of aggregating.
    #[arg(long = "session", short = 's')]
    pub session: Option<String>,

    /// Output as JSON instead of a formatted table.
    #[arg(long = "json")]
    pub json: bool,
}

// ---------------------------------------------------------------------------
// Model pricing (hardcoded defaults — opencode stores model in session data)
// ---------------------------------------------------------------------------

/// Hardcoded token pricing for common models (USD per 1M tokens).
fn token_cost_per_million(model: &str) -> Option<(f64, f64)> {
    // (input_cost_per_million, output_cost_per_million)
    match model {
        // GPT-4o family
        m if m.contains("gpt-4o") && !m.contains("mini") => Some((5.00, 15.00)),
        m if m.contains("gpt-4o-mini") => Some((0.15, 0.60)),
        // GPT-4 Turbo
        m if m.contains("gpt-4-turbo") || m.contains("gpt-4-0125") => Some((10.00, 30.00)),
        // GPT-4
        m if m.contains("gpt-4") && !m.contains("4-turbo") => Some((30.00, 60.00)),
        // GPT-3.5
        m if m.contains("gpt-3.5-turbo") => Some((0.50, 1.50)),
        // Claude 3.5
        m if m.contains("claude-3.5") && m.contains("sonnet") => Some((3.00, 15.00)),
        // Claude 3 Opus / Sonnet / Haiku
        m if m.contains("claude-3-opus") => Some((15.00, 75.00)),
        m if m.contains("claude-3-sonnet") => Some((3.00, 15.00)),
        m if m.contains("claude-3-haiku") => Some((0.25, 1.25)),
        // Claude 2
        m if m.contains("claude-2") => Some((8.00, 24.00)),
        // O1-preview / O1-mini
        m if m.contains("o1-preview") => Some((15.00, 60.00)),
        m if m.contains("o1-mini") => Some((3.00, 12.00)),
        // O3 / O4 family
        m if m.contains("o3") || m.contains("o4") => Some((15.00, 60.00)),
        // Gemini
        m if m.contains("gemini-2") && m.contains("flash") => Some((0.10, 0.40)),
        m if m.contains("gemini-1.5") && m.contains("flash") => Some((0.075, 0.30)),
        m if m.contains("gemini-1.5") && m.contains("pro") => Some((3.50, 10.50)),
        // Default — use GPT-4o pricing as fallback
        _ => Some((5.00, 15.00)),
    }
}

/// Session-level token statistics.
#[derive(Debug, serde::Serialize)]
pub struct SessionTokenStats {
    pub session_id: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub cost_usd: f64,
    pub message_count: i64,
    pub finish_reasons: BTreeMap<String, i64>,
}

/// Grand totals across multiple sessions.
#[derive(Debug, serde::Serialize)]
pub struct TokenStatsSummary {
    pub sessions_count: usize,
    pub grand_total: GrandTotals,
    pub per_session: Vec<SessionTokenStats>,
}

#[derive(Debug, serde::Serialize)]
pub struct GrandTotals {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub cost_usd: f64,
    pub message_count: i64,
    pub finish_reasons: BTreeMap<String, i64>,
    // Averages
    pub avg_input_tokens: f64,
    pub avg_output_tokens: f64,
    pub avg_reasoning_tokens: f64,
    pub avg_cost_usd: f64,
}

/// Run the token-stats subcommand.
#[cfg(feature = "scripts")]
pub fn run(args: &Args) -> anyhow::Result<()> {
    let db = OpencodeDb::open(None)?;

    let sessions = if let Some(session_id) = &args.session {
        let sess = db.get_session(session_id)?;
        sess.map(|s| vec![s]).unwrap_or_default()
    } else {
        db.list_sessions(None, args.sessions)?
    };

    let per_session: Vec<SessionTokenStats> = sessions
        .iter()
        .map(|session| compute_session_stats(session, &db))
        .collect::<Result<Vec<_>, _>>()?;

    if args.json {
        let sessions_count = per_session.len();
        let grand = aggregate_totals(&per_session);
        let summary = TokenStatsSummary {
            sessions_count,
            grand_total: grand,
            per_session,
        };
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        print_table(&per_session);
    }

    Ok(())
}

/// Run the token-stats subcommand (stub when `scripts` feature is disabled).
#[cfg(not(feature = "scripts"))]
pub fn run(_args: &Args) -> anyhow::Result<()> {
    anyhow::bail!("token-stats requires the `scripts` feature (run with --features scripts)")
}

#[cfg(feature = "scripts")]
fn compute_session_stats(
    session: &crate::commands::db::SessionRow,
    db: &OpencodeDb,
) -> anyhow::Result<SessionTokenStats> {
    let messages = db.get_session_messages(&session.id)?;
    let mut input_tokens: i64 = 0;
    let mut output_tokens: i64 = 0;
    let mut reasoning_tokens: i64 = 0;
    let mut finish_reasons: BTreeMap<String, i64> = BTreeMap::new();

    // Extract model ID from session.model JSON (e.g. '{"id":"gpt-4o","providerID":"openai"}')
    let model = session
        .model
        .as_ref()
        .and_then(|m| serde_json::from_str::<serde_json::Value>(m).ok())
        .and_then(|v| v.get("id").and_then(|id| id.as_str()).map(String::from))
        .unwrap_or_else(|| "unknown".to_string());

    for msg in &messages {
        // Use the per-message token breakdown from message.data["tokens"].
        // For assistant messages this is populated; for user messages it is 0.
        if msg.tokens_input > 0 || msg.tokens_output > 0 {
            input_tokens += msg.tokens_input;
            output_tokens += msg.tokens_output;
            reasoning_tokens += msg.tokens_reasoning;
        } else {
            // Fallback: distribute token_count by role
            match msg.role.as_str() {
                "user" | "system" => input_tokens += msg.token_count,
                "assistant" => output_tokens += msg.token_count,
                _ => input_tokens += msg.token_count,
            }
        }

        // Also add reasoning tokens from parts that have type "reasoning"
        for part in &msg.parts {
            if part.part_type == "reasoning" {
                if let Some(text) = part.text.as_ref() {
                    reasoning_tokens += text.chars().count() as i64 / 4;
                }
            }
        }

        if let Some(ref fr) = msg.finish_reason {
            *finish_reasons.entry(fr.clone()).or_insert(0) += 1;
        }
    }

    // Cache tokens are direct columns on the session table
    let cache_read_tokens: i64 = session.tokens_cache_read;
    let cache_write_tokens: i64 = session.tokens_cache_write;

    // Compute cost
    let (input_cost, output_cost) = token_cost_per_million(&model).unwrap_or((5.0, 15.0));
    let cost_usd = (input_tokens as f64 * input_cost / 1_000_000.0)
        + (output_tokens as f64 * output_cost / 1_000_000.0);

    Ok(SessionTokenStats {
        session_id: session.id.clone(),
        input_tokens,
        output_tokens,
        reasoning_tokens,
        cache_read_tokens,
        cache_write_tokens,
        cost_usd,
        message_count: messages.len() as i64,
        finish_reasons,
    })
}

#[cfg(feature = "scripts")]
fn aggregate_totals(sessions: &[SessionTokenStats]) -> GrandTotals {
    let mut input_tokens = 0;
    let mut output_tokens = 0;
    let mut reasoning_tokens = 0;
    let mut cache_read_tokens = 0;
    let mut cache_write_tokens = 0;
    let mut cost_usd = 0.0;
    let mut message_count = 0;
    let mut finish_reasons: BTreeMap<String, i64> = BTreeMap::new();

    for s in sessions {
        input_tokens += s.input_tokens;
        output_tokens += s.output_tokens;
        reasoning_tokens += s.reasoning_tokens;
        cache_read_tokens += s.cache_read_tokens;
        cache_write_tokens += s.cache_write_tokens;
        cost_usd += s.cost_usd;
        message_count += s.message_count;
        for (k, v) in &s.finish_reasons {
            *finish_reasons.entry(k.clone()).or_insert(0) += v;
        }
    }

    let n = sessions.len() as f64;
    GrandTotals {
        input_tokens,
        output_tokens,
        reasoning_tokens,
        cache_read_tokens,
        cache_write_tokens,
        cost_usd,
        message_count,
        finish_reasons,
        avg_input_tokens: if n > 0.0 {
            input_tokens as f64 / n
        } else {
            0.0
        },
        avg_output_tokens: if n > 0.0 {
            output_tokens as f64 / n
        } else {
            0.0
        },
        avg_reasoning_tokens: if n > 0.0 {
            reasoning_tokens as f64 / n
        } else {
            0.0
        },
        avg_cost_usd: if n > 0.0 { cost_usd / n } else { 0.0 },
    }
}

#[cfg(feature = "scripts")]
fn print_table(sessions: &[SessionTokenStats]) {
    let n = sessions.len();
    println!(
        "=== Token Statistics ({} session{}) ===",
        n,
        if n == 1 { "" } else { "s" }
    );

    if sessions.is_empty() {
        println!("  (no sessions)");
        return;
    }

    // Header
    println!();
    println!(
        "  {:<22} | {:>10} | {:>10} | {:>10} | {:>10} | {:>10} | {:>7}",
        "Session", "Input", "Output", "Reasoning", "CacheRD", "CacheWR", "Cost($)"
    );
    println!(
        "  {:─<22}───{:─<10}───{:─<10}───{:─<10}───{:─<10}───{:─<10}───{:─<7}",
        "", "", "", "", "", "", ""
    );

    let grand = aggregate_totals(sessions);

    for s in sessions {
        println!(
            "  {:<22} | {:>10} | {:>10} | {:>10} | {:>10} | {:>10} | {:>7.3}",
            truncate(&s.session_id, 22),
            s.input_tokens,
            s.output_tokens,
            s.reasoning_tokens,
            s.cache_read_tokens,
            s.cache_write_tokens,
            s.cost_usd
        );
    }

    // Grand totals
    println!(
        "  {:─<22}───{:─<10}───{:─<10}───{:─<10}───{:─<10}───{:─<10}───{:─<7}",
        "", "", "", "", "", "", ""
    );
    println!(
        "  {:<22} | {:>10} | {:>10} | {:>10} | {:>10} | {:>10} | {:>7.3}",
        "GRAND TOTAL",
        grand.input_tokens,
        grand.output_tokens,
        grand.reasoning_tokens,
        grand.cache_read_tokens,
        grand.cache_write_tokens,
        grand.cost_usd
    );
    println!(
        "  {:<22} | {:>10.1} | {:>10.1} | {:>10} | {:>10} | {:>10} | {:>7.3}",
        "AVG PER SESSION",
        grand.avg_input_tokens,
        grand.avg_output_tokens,
        grand.avg_reasoning_tokens,
        grand.cache_read_tokens,
        grand.cache_write_tokens,
        grand.avg_cost_usd
    );
}

fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() > max_chars {
        let truncated: String = s.chars().take(max_chars.saturating_sub(3)).collect();
        format!("{}...", truncated)
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_stats_args_parsing_default() {
        let args = Args::parse_from(["token-stats"]);
        assert_eq!(args.sessions, 10);
        assert!(args.session.is_none());
        assert!(!args.json);
    }

    #[test]
    fn token_stats_args_sessions() {
        let args = Args::parse_from(["token-stats", "--sessions", "25"]);
        assert_eq!(args.sessions, 25);
    }

    #[test]
    fn token_stats_args_session_flag() {
        let args = Args::parse_from(["token-stats", "--session", "sess-abc123"]);
        assert_eq!(args.session, Some("sess-abc123".to_string()));
    }

    #[test]
    fn token_stats_args_json_flag() {
        let args = Args::parse_from(["token-stats", "--json"]);
        assert!(args.json);
    }

    #[test]
    fn token_stats_args_combined() {
        let args = Args::parse_from([
            "token-stats",
            "--sessions",
            "5",
            "--session",
            "sess-xyz",
            "--json",
        ]);
        assert_eq!(args.sessions, 5);
        assert_eq!(args.session, Some("sess-xyz".to_string()));
        assert!(args.json);
    }

    #[test]
    fn token_cost_per_million_gpt4o() {
        let cost = token_cost_per_million("gpt-4o");
        assert_eq!(cost, Some((5.0, 15.0)));
    }

    #[test]
    fn token_cost_per_million_o1_preview() {
        let cost = token_cost_per_million("o1-preview");
        assert_eq!(cost, Some((15.0, 60.0)));
    }

    #[test]
    fn token_cost_per_million_unknown() {
        // Unknown models fall back to GPT-4o pricing
        let cost = token_cost_per_million("unknown-model-xyz");
        assert_eq!(cost, Some((5.0, 15.0)));
    }

    #[test]
    fn truncate_function() {
        assert_eq!(truncate("short", 10), "short");
        assert_eq!(truncate("this is a long string", 10), "this is...");
        assert_eq!(truncate("exact", 5), "exact");
    }
}
