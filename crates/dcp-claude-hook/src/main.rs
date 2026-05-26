//! `dcp-claude-hook` — Claude Code SessionStart hook binary.
//!
//! Runs on Claude Code session start to prepare the context pruner state.
//! Place in `~/.claude/commands/` directory as `session-start-dcp.sh` (or similar hook).
//!
//! Usage:
//!   Copy this binary to a location in your PATH, then create a Claude Code hook
//!   that invokes it on session start.

use std::env;
use std::process;

fn main() -> anyhow::Result<()> {
    // Allow debug output via DCP_DEBUG env var
    let debug = env::var("DCP_DEBUG").is_ok();

    if debug {
        eprintln!("[dcp-claude-hook] Starting...");
    }

    // Load configuration to verify it works
    match dcp_config::Config::load_default() {
        Ok(config) => {
            if debug {
                eprintln!("[dcp-claude-hook] Config loaded: enabled={}, debug={}",
                    config.enabled, config.debug);
            }

            // Verify core library is functional by creating a ContextPruner
            match dcp_core::ContextPruner::new(config) {
                Ok(_) => {
                    if debug {
                        eprintln!("[dcp-claude-hook] ContextPruner initialized OK");
                    }
                    process::exit(0);
                }
                Err(e) => {
                    eprintln!("[dcp-claude-hook] ERROR: ContextPruner init failed: {:?}", e);
                    process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("[dcp-claude-hook] WARNING: Config load failed (using defaults): {:?}", e);
            // Continue with defaults - non-fatal
            process::exit(0);
        }
    }
}
