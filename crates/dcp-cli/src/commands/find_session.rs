//! `find-session` subcommand — find sessions by pattern or date range.

use clap::Parser;
use dcp_storage::{default_storage_dir, FileStateStore};
use dcp_traits::StatePersistence;

/// Find sessions by ID pattern or date range.
#[derive(Parser, Debug)]
pub struct Args {
    /// Glob pattern to match session IDs against.
    #[arg(long = "pattern", short = 'p')]
    pub pattern: Option<String>,

    /// Find sessions after this date (YYYY-MM-DD).
    #[arg(long = "after", short = 'a')]
    pub after: Option<String>,

    /// Find sessions before this date (YYYY-MM-DD).
    #[arg(long = "before", short = 'b')]
    pub before: Option<String>,
}

/// Run the find-session subcommand.
pub fn run(args: &Args) -> anyhow::Result<()> {
    let store = FileStateStore::new(default_storage_dir());

    let all_sessions = store
        .list_sessions()
        .map_err(|e| anyhow::anyhow!("failed to list sessions: {}", e))?;

    let after_timestamp = args.after.as_ref().and_then(|s| parse_date(s).ok());
    let before_timestamp = args.before.as_ref().and_then(|s| parse_date(s).ok());

    let matching: Vec<&String> = all_sessions
        .iter()
        .filter(|session_id| {
            if let Some(pattern) = &args.pattern {
                if !matches_glob(session_id, pattern) {
                    return false;
                }
            }

            if after_timestamp.is_some() || before_timestamp.is_some() {
                if let Ok(Some(persisted)) = store.load(session_id) {
                    let timestamp = extract_last_updated(&persisted);
                    if let Some(after) = after_timestamp {
                        if timestamp < after {
                            return false;
                        }
                    }
                    if let Some(before) = before_timestamp {
                        if timestamp > before {
                            return false;
                        }
                    }
                } else {
                    return false;
                }
            }

            true
        })
        .collect();

    if matching.is_empty() {
        println!("No sessions found matching the criteria.");
    } else {
        println!("Found {} session(s):", matching.len());
        for session_id in &matching {
            if let Ok(Some(persisted)) = store.load(session_id) {
                let last_updated = extract_last_updated_string(&persisted);
                let current_turn = extract_current_turn(&persisted);
                println!(
                    "  {} (updated: {}, turn: {})",
                    session_id, last_updated, current_turn
                );
            } else {
                println!("  {}", session_id);
            }
        }
    }

    Ok(())
}

fn matches_glob(text: &str, pattern: &str) -> bool {
    let text_chars: Vec<char> = text.chars().collect();
    let pattern_chars: Vec<char> = pattern.chars().collect();
    matches_glob_inner(&text_chars, 0, &pattern_chars, 0)
}

fn matches_glob_inner(text: &[char], ti: usize, pattern: &[char], pi: usize) -> bool {
    if pi >= pattern.len() {
        return ti >= text.len();
    }

    let p = pattern[pi];
    if p == '*' {
        if pi + 1 >= pattern.len() {
            return true;
        }
        for i in ti..=text.len() {
            if matches_glob_inner(text, i, pattern, pi + 1) {
                return true;
            }
        }
        return false;
    }

    if p == '?' {
        if ti >= text.len() {
            return false;
        }
        return matches_glob_inner(text, ti + 1, pattern, pi + 1);
    }

    if ti >= text.len() {
        return false;
    }

    if text[ti] == p {
        return matches_glob_inner(text, ti + 1, pattern, pi + 1);
    }

    false
}

fn parse_date(s: &str) -> anyhow::Result<i64> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 {
        anyhow::bail!("date must be in YYYY-MM-DD format");
    }

    let year: i64 = parts[0].parse().map_err(|_| anyhow::anyhow!("invalid year"))?;
    let month: u8 = parts[1].parse().map_err(|_| anyhow::anyhow!("invalid month"))?;
    let day: u8 = parts[2].parse().map_err(|_| anyhow::anyhow!("invalid day"))?;

    if !(1..=12).contains(&month) {
        anyhow::bail!("month must be between 1 and 12");
    }
    if !(1..=31).contains(&day) {
        anyhow::bail!("day must be between 1 and 31");
    }

    let days = days_from_epoch(year, month, day);
    Ok(days * 86400)
}

fn days_from_epoch(year: i64, month: u8, day: u8) -> i64 {
    let mut days = (year - 1970) * 365 + ((year - 1969) / 4) - ((year - 1901) / 100)
        + ((year - 1601) / 400);

    let month_days = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 1..month {
        days += month_days[(m - 1) as usize] as i64;
    }

    if month > 2 && is_leap_year(year) {
        days += 1;
    }

    days + (day as i64) - 1
}

fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

fn extract_last_updated(persisted: &dcp_traits::PersistedState) -> i64 {
    match persisted {
        dcp_traits::PersistedState::V1(v1) => {
            let ts = v1.last_updated.trim();
            if ts.len() >= 10 {
                parse_date(&ts[..10]).unwrap_or(0)
            } else {
                0
            }
        }
    }
}

fn extract_last_updated_string(persisted: &dcp_traits::PersistedState) -> String {
    match persisted {
        dcp_traits::PersistedState::V1(v1) => v1.last_updated.clone(),
    }
}

fn extract_current_turn(persisted: &dcp_traits::PersistedState) -> u32 {
    match persisted {
        dcp_traits::PersistedState::V1(v1) => v1.current_turn,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_session_args_parsing_pattern() {
        let args = Args::parse_from(["find-session", "--pattern", "test-*"]);
        assert_eq!(args.pattern, Some("test-*".to_string()));
        assert!(args.after.is_none());
        assert!(args.before.is_none());
    }

    #[test]
    fn find_session_args_parsing_date_range() {
        let args =
            Args::parse_from(["find-session", "--after", "2024-01-01", "--before", "2024-12-31"]);
        assert!(args.pattern.is_none());
        assert_eq!(args.after, Some("2024-01-01".to_string()));
        assert_eq!(args.before, Some("2024-12-31".to_string()));
    }

    #[test]
    fn find_session_args_parsing_all() {
        let args = Args::parse_from([
            "find-session",
            "--pattern",
            "session-*",
            "--after",
            "2024-01-01",
        ]);
        assert_eq!(args.pattern, Some("session-*".to_string()));
        assert_eq!(args.after, Some("2024-01-01".to_string()));
        assert!(args.before.is_none());
    }

    #[test]
    fn glob_matching_basic() {
        assert!(matches_glob("test-session", "test-*"));
        assert!(matches_glob("test-session", "test-session"));
        assert!(!matches_glob("other-session", "test-*"));
    }

    #[test]
    fn glob_matching_question_mark() {
        assert!(matches_glob("test-1", "test-?"));
        assert!(matches_glob("test-a", "test-?"));
        assert!(!matches_glob("test-12", "test-?"));
    }
}