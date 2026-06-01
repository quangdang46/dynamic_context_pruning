//! SQLite data access layer for opencode's session database.
//!
//! This module provides a typed interface to the opencode SQLite database
//! (typically at `~/.local/share/opencode/opencode.db`). It is feature-gated
//! behind the `scripts` feature — existing commands (stats, find-session,
//! timeline) use `FileStateStore` and do not depend on this module.
//!
//! # Actual opencode Schema (verified against live DB)
//!
//! ```sql
//! CREATE TABLE project (
//!     id TEXT PRIMARY KEY,
//!     directory TEXT,
//!     data TEXT  -- JSON blob
//! );
//!
//! CREATE TABLE session (
//!     id TEXT PRIMARY KEY,
//!     project_id TEXT,
//!     parent_id TEXT,
//!     slug TEXT,
//!     directory TEXT,
//!     title TEXT,                  -- session title (may contain model info)
//!     version TEXT,
//!     share_url TEXT,
//!     summary_additions INTEGER,
//!     summary_deletions INTEGER,
//!     summary_files INTEGER,
//!     summary_diffs TEXT,
//!     revert TEXT,
//!     permission TEXT,
//!     time_created INTEGER,          -- unix timestamp ms
//!     time_updated INTEGER,          -- unix timestamp ms
//!     time_compacting INTEGER,
//!     time_archived INTEGER,
//!     workspace_id TEXT,
//!     path TEXT,
//!     agent TEXT,                   -- agent name (e.g. "build")
//!     model TEXT,                   -- JSON: {"id":"...","providerID":"..."}
//!     cost REAL,
//!     tokens_input INTEGER,
//!     tokens_output INTEGER,
//!     tokens_reasoning INTEGER,
//!     tokens_cache_read INTEGER,
//!     tokens_cache_write INTEGER
//! );
//!
//! CREATE TABLE message (
//!     id TEXT PRIMARY KEY,
//!     session_id TEXT,
//!     time_created INTEGER,          -- unix timestamp ms
//!     time_updated INTEGER,
//!     data TEXT                     -- JSON: role, model, tokens, finish, etc.
//! );
//!
//! CREATE TABLE part (
//!     id TEXT PRIMARY KEY,
//!     message_id TEXT,
//!     session_id TEXT,
//!     time_created INTEGER,
//!     time_updated INTEGER,
//!     data TEXT                     -- JSON: {"type":"text"|"reasoning"|..., "text":...}
//! );
//! ```

use serde::{Deserialize, Serialize};

#[cfg(feature = "scripts")]
use rusqlite::{Connection, params};

/// Directory where opencode stores its data on Linux/macOS.
fn default_opencode_db_path() -> std::path::PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| "~".to_string());
    let mut path = std::path::PathBuf::from(home);
    path.push(".local");
    path.push("share");
    path.push("opencode");
    path.push("opencode.db");
    path
}

// ---------------------------------------------------------------------------
// Row types
// ---------------------------------------------------------------------------

/// A row from the `project` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ProjectRow {
    pub id: String,
    pub worktree: String,
    pub vcs: String,
    pub name: String,
    pub icon_url: Option<String>,
    pub icon_color: Option<String>,
    pub time_created: i64,
    pub time_updated: i64,
    pub time_initialized: i64,
    pub sandboxes: Option<String>,
    pub commands: Option<String>,
    pub icon_url_override: Option<String>,
}

/// A row from the `session` table.
///
/// Fields are mapped from the actual opencode `session` table columns:
/// `id, project_id, directory, title, agent, model, cost,
///  tokens_input, tokens_output, tokens_reasoning,
///  tokens_cache_read, tokens_cache_write,
///  time_created, time_updated, data`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRow {
    /// Session UUID.
    pub id: String,
    /// Parent project UUID (if set).
    pub project_id: Option<String>,
    /// Working directory associated with this session.
    pub directory: Option<String>,
    /// Session title (may contain model info).
    pub title: Option<String>,
    /// Agent name (e.g. "build").
    pub agent: Option<String>,
    /// Model info as JSON string (e.g. `'{"id":"gpt-4o","providerID":"openai"}'`).
    pub model: Option<String>,
    /// Total cost in USD.
    pub cost: f64,
    /// Input token count.
    pub tokens_input: i64,
    /// Output token count.
    pub tokens_output: i64,
    /// Reasoning token count.
    pub tokens_reasoning: i64,
    /// Cache read token count.
    pub tokens_cache_read: i64,
    /// Cache write token count.
    pub tokens_cache_write: i64,
    /// Unix timestamp (ms) when the session was created.
    pub time_created: i64,
    /// Unix timestamp (ms) of the last update.
    pub time_updated: i64,
}

/// A message with its associated parts from the `part` table.
///
/// Role, token count, and finish reason are extracted from `message.data` JSON:
/// ```json
/// {
///   "role": "user"|"assistant",
///   "tokens": {"total": N, "input": N, "output": N, "reasoning": N, "cache": {...}},
///   "finish": "stop"|"tool-calls"|null,
///   ...
/// }
/// ```
///
/// Parts are loaded from the `part` table via LEFT JOIN. Each `part.data` JSON:
/// ```json
/// {"type": "text"|"reasoning"|"step-start"|..., "text": "..."}
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageWithParts {
    /// Message UUID.
    pub message_id: String,
    /// Parent session UUID.
    pub session_id: String,
    /// Role extracted from `message.data["role"]`: `user`, `assistant`, `system`.
    pub role: String,
    /// Deserialized parts from the `part` table (one entry per part row).
    /// Each `PartData` has `type` and `text`/`snapshot` fields.
    pub parts: Vec<PartData>,
    /// Token count extracted from `message.data["tokens"]["total"]`.
    pub token_count: i64,
    /// Input token count from `message.data["tokens"]["input"]` (0 if not present).
    pub tokens_input: i64,
    /// Output token count from `message.data["tokens"]["output"]` (0 if not present).
    pub tokens_output: i64,
    /// Reasoning token count from `message.data["tokens"]["reasoning"]` (0 if not present).
    pub tokens_reasoning: i64,
    /// Finish reason extracted from `message.data["finish"]`.
    pub finish_reason: Option<String>,
    /// Unix timestamp (ms) when the message was created.
    pub time_created: i64,
}

/// A single part from the `part` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartData {
    /// Part type: "text", "reasoning", "step-start", etc.
    #[serde(rename = "type")]
    pub part_type: String,
    /// Text content (present for "text" and "reasoning" types).
    pub text: Option<String>,
    /// Snapshot (present for "step-start" type).
    pub snapshot: Option<String>,
    /// Raw JSON data for any additional fields.
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

// ---------------------------------------------------------------------------
// Database handle
// ---------------------------------------------------------------------------

/// Handle to an opencode SQLite database.
#[derive(Debug)]
pub struct OpencodeDb {
    #[cfg(feature = "scripts")]
    conn: Connection,
}

impl OpencodeDb {
    /// Open (or create) the database at `path`.
    ///
    /// If `path` is `None`, resolves to `~/.local/share/opencode/opencode.db`.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be opened or is not a valid SQLite DB.
    #[cfg(feature = "scripts")]
    pub fn open(path: Option<&str>) -> anyhow::Result<Self> {
        let db_path = path
            .map(std::path::PathBuf::from)
            .unwrap_or_else(default_opencode_db_path);

        let conn = Connection::open(&db_path)
            .map_err(|e| anyhow::anyhow!("failed to open opencode DB at {:?}: {}", db_path, e))?;

        Ok(OpencodeDb { conn })
    }

    // -------------------------------------------------------------------------
    // Project queries
    // -------------------------------------------------------------------------

    /// List all projects.
    #[cfg(feature = "scripts")]
    #[allow(dead_code)]
    pub fn list_projects(&self) -> anyhow::Result<Vec<ProjectRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, worktree, vcs, name, icon_url, icon_color, \
             time_created, time_updated, time_initialized, \
             sandboxes, commands, icon_url_override \
             FROM project ORDER BY id",
        )?;

        let rows = stmt.query_map([], |row| {
            Ok(ProjectRow {
                id: row.get(0)?,
                worktree: row.get(1)?,
                vcs: row.get(2)?,
                name: row.get(3)?,
                icon_url: row.get(4)?,
                icon_color: row.get(5)?,
                time_created: row.get(6)?,
                time_updated: row.get(7)?,
                time_initialized: row.get(8)?,
                sandboxes: row.get(9)?,
                commands: row.get(10)?,
                icon_url_override: row.get(11)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("list_projects error: {}", e))
    }

    // -------------------------------------------------------------------------
    // Session queries
    // -------------------------------------------------------------------------

    /// List sessions, optionally filtered by directory.
    ///
    /// Queries actual opencode session columns:
    /// `id, project_id, directory, title, agent, model, cost,
    ///  tokens_input, tokens_output, tokens_reasoning,
    ///  tokens_cache_read, tokens_cache_write,
    ///  time_created, time_updated`
    #[cfg(feature = "scripts")]
    pub fn list_sessions(
        &self,
        directory: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<SessionRow>> {
        let sql = if directory.is_some() {
            "SELECT id, project_id, directory, title, agent, model, cost, \
             tokens_input, tokens_output, tokens_reasoning, \
             tokens_cache_read, tokens_cache_write, \
             time_created, time_updated \
             FROM session WHERE directory = ?1 ORDER BY time_updated DESC LIMIT ?2"
        } else {
            "SELECT id, project_id, directory, title, agent, model, cost, \
             tokens_input, tokens_output, tokens_reasoning, \
             tokens_cache_read, tokens_cache_write, \
             time_created, time_updated \
             FROM session ORDER BY time_updated DESC LIMIT ?1"
        };

        let mut stmt = self.conn.prepare(sql)?;

        let rows = if let Some(dir) = directory {
            stmt.query_map(params![dir, limit as i64], session_row_map)?
        } else {
            stmt.query_map(params![limit as i64], session_row_map)?
        };

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("list_sessions error: {}", e))
    }

    /// Get a single session by ID.
    #[cfg(feature = "scripts")]
    pub fn get_session(&self, session_id: &str) -> anyhow::Result<Option<SessionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, directory, title, agent, model, cost, \
             tokens_input, tokens_output, tokens_reasoning, \
             tokens_cache_read, tokens_cache_write, \
             time_created, time_updated \
             FROM session WHERE id = ?1",
        )?;

        let mut rows = stmt.query_map(params![session_id], session_row_map)?;

        match rows.next() {
            Some(result) => Ok(Some(result?)),
            None => Ok(None),
        }
        .map_err(|e: rusqlite::Error| anyhow::anyhow!("get_session error: {}", e))
    }

    // -------------------------------------------------------------------------
    // Message queries (JOIN with part table)
    // -------------------------------------------------------------------------

    /// Get all messages for a session with their parts, ordered by creation time.
    ///
    /// Does a LEFT JOIN with the `part` table to load all parts per message.
    /// Multiple parts for the same message are aggregated into `msg.parts`.
    #[cfg(feature = "scripts")]
    pub fn get_session_messages(&self, session_id: &str) -> anyhow::Result<Vec<MessageWithParts>> {
        // Join message + part tables, aggregate parts per message
        let mut stmt = self.conn.prepare(
            "SELECT \
                m.id, m.session_id, m.time_created, m.data, \
                p.data \
             FROM message m \
             LEFT JOIN part p ON m.id = p.message_id \
             WHERE m.session_id = ?1 \
             ORDER BY m.time_created ASC, p.time_created ASC",
        )?;

        let rows = stmt.query_map(params![session_id], message_row_map)?;

        // Aggregate parts per message (group by message_id)
        aggregate_messages(rows).map_err(|e| anyhow::anyhow!("get_session_messages error: {}", e))
    }

    /// Get a single message within a session by message ID.
    #[cfg(feature = "scripts")]
    pub fn get_session_message(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> anyhow::Result<Option<MessageWithParts>> {
        let mut stmt = self.conn.prepare(
            "SELECT \
                m.id, m.session_id, m.time_created, m.data, \
                p.data \
             FROM message m \
             LEFT JOIN part p ON m.id = p.message_id \
             WHERE m.session_id = ?1 AND m.id = ?2 \
             ORDER BY m.time_created ASC, p.time_created ASC",
        )?;

        let rows = stmt.query_map(params![session_id, message_id], message_row_map)?;

        match aggregate_messages(rows) {
            Ok(mut msgs) => Ok(msgs.pop()),
            Err(e) => Err(e),
        }
    }
}

// ---------------------------------------------------------------------------
// Row mappers
// ---------------------------------------------------------------------------

/// Map a session row (15 columns + data) to SessionRow.
#[cfg(feature = "scripts")]
fn session_row_map(row: &rusqlite::Row) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        id: row.get(0)?,
        project_id: row.get(1)?,
        directory: row.get(2)?,
        title: row.get(3)?,
        agent: row.get(4)?,
        model: row.get(5)?,
        cost: row.get(6)?,
        tokens_input: row.get(7)?,
        tokens_output: row.get(8)?,
        tokens_reasoning: row.get(9)?,
        tokens_cache_read: row.get(10)?,
        tokens_cache_write: row.get(11)?,
        time_created: row.get(12)?,
        time_updated: row.get(13)?,
    })
}

/// Map a single (message, part) row to `RawMessageRow`.
#[cfg(feature = "scripts")]
fn message_row_map(row: &rusqlite::Row) -> rusqlite::Result<RawMessageRow> {
    let data_str: String = row.get(3)?;
    let data: serde_json::Value =
        serde_json::from_str(&data_str).unwrap_or(serde_json::Value::Null);
    let part_data_str: Option<String> = row.get(4)?;
    let part_data: Option<PartData> = part_data_str.and_then(|s| serde_json::from_str(&s).ok());

    Ok(RawMessageRow {
        message_id: row.get(0)?,
        session_id: row.get(1)?,
        time_created: row.get(2)?,
        message_data: data,
        part: part_data,
    })
}

/// Intermediate row type for aggregating multiple part rows per message.
#[cfg(feature = "scripts")]
struct RawMessageRow {
    message_id: String,
    session_id: String,
    time_created: i64,
    message_data: serde_json::Value,
    part: Option<PartData>,
}

/// Aggregate rows from the (message LEFT JOIN part) query into `MessageWithParts`.
///
/// Each call returns one (message, part) pair. Multiple rows with the same
/// `message_id` represent multiple parts of the same message. We aggregate
/// them into a single `MessageWithParts` with a `Vec<PartData>`.
#[cfg(feature = "scripts")]
fn aggregate_messages(
    rows: impl Iterator<Item = Result<RawMessageRow, rusqlite::Error>>,
) -> Result<Vec<MessageWithParts>, anyhow::Error> {
    let mut messages: Vec<MessageWithParts> = Vec::new();
    let mut current_id: Option<String> = None;
    let mut current_msg: Option<MessageWithParts> = None;

    for row_result in rows {
        let raw = row_result?;

        if current_id.as_ref() != Some(&raw.message_id) {
            // New message — push the previous one and start a new entry
            if let Some(msg) = current_msg.take() {
                messages.push(msg);
            }

            // Extract role from message.data["role"]
            let role = raw
                .message_data
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();

            // Extract token_count from message.data["tokens"]["total"]
            let tokens_json = raw.message_data.get("tokens");
            let token_count = tokens_json
                .and_then(|v| v.get("total"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let tokens_input = tokens_json
                .and_then(|v| v.get("input"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let tokens_output = tokens_json
                .and_then(|v| v.get("output"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let tokens_reasoning = tokens_json
                .and_then(|v| v.get("reasoning"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);

            // Extract finish_reason from message.data["finish"]
            let finish_reason = raw
                .message_data
                .get("finish")
                .and_then(|v| v.as_str())
                .map(String::from);

            let mut parts = Vec::new();
            if let Some(p) = raw.part {
                parts.push(p);
            }

            current_msg = Some(MessageWithParts {
                message_id: raw.message_id.clone(),
                session_id: raw.session_id.clone(),
                role,
                parts,
                token_count,
                tokens_input,
                tokens_output,
                tokens_reasoning,
                finish_reason,
                time_created: raw.time_created,
            });
            current_id = Some(raw.message_id);
        } else {
            // Same message, additional part
            if let Some(ref mut msg) = current_msg {
                if let Some(p) = raw.part {
                    msg.parts.push(p);
                }
            }
        }
    }

    // Don't forget the last message
    if let Some(msg) = current_msg {
        messages.push(msg);
    }

    Ok(messages)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(feature = "scripts")]
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_creates_new_db() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test_opencode.db");

        // Opening a non-existent DB should succeed (creates an empty file)
        let db = OpencodeDb::open(Some(db_path.to_str().unwrap()));
        assert!(db.is_ok());
    }

    #[test]
    fn insert_and_retrieve_project() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test_opencode.db");
        let db = OpencodeDb::open(Some(db_path.to_str().unwrap())).unwrap();

        // Create the project table with actual schema
        db.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS project (id TEXT PRIMARY KEY, worktree TEXT, vcs TEXT, name TEXT, icon_url TEXT, icon_color TEXT, time_created INTEGER, time_updated INTEGER, time_initialized INTEGER, sandboxes TEXT, commands TEXT, icon_url_override TEXT)",
                [],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO project (id, worktree, vcs, name, icon_url, icon_color, time_created, time_updated, time_initialized, sandboxes, commands, icon_url_override) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params!["proj-1", "/tmp/test", "", "", "", "", 0, 0, 0, "", "", ""],
            )
            .unwrap();

        let projects = db.list_projects().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id, "proj-1");
        assert_eq!(projects[0].worktree, "/tmp/test");
    }

    #[test]
    fn insert_and_retrieve_session() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test_opencode.db");
        let db = OpencodeDb::open(Some(db_path.to_str().unwrap())).unwrap();

        // Create session table with ACTUAL opencode schema columns
        db.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS session (\
                 id TEXT PRIMARY KEY, project_id TEXT, directory TEXT, \
                 title TEXT, agent TEXT, model TEXT, cost REAL, \
                 tokens_input INTEGER, tokens_output INTEGER, tokens_reasoning INTEGER, \
                 tokens_cache_read INTEGER, tokens_cache_write INTEGER, \
                 time_created INTEGER, time_updated INTEGER, data TEXT)",
                [],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO session \
                 (id, project_id, directory, title, agent, model, cost, \
                  tokens_input, tokens_output, tokens_reasoning, \
                  tokens_cache_read, tokens_cache_write, \
                  time_created, time_updated, data) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    "sess-1",
                    "proj-1",
                    "/tmp/test",
                    "Test Session",
                    "build",
                    r#"{"id":"gpt-4o","providerID":"openai"}"#,
                    0.05,
                    1000_i64,
                    500_i64,
                    100_i64,
                    0_i64,
                    0_i64,
                    1718000000000_i64,
                    1718003600000_i64,
                    r#"{"extra":"field"}"#
                ],
            )
            .unwrap();

        let sessions = db.list_sessions(None, 50).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "sess-1");
        assert_eq!(sessions[0].project_id, Some("proj-1".to_string()));
        assert_eq!(sessions[0].title, Some("Test Session".to_string()));
        assert_eq!(sessions[0].agent, Some("build".to_string()));
        assert_eq!(
            sessions[0].model,
            Some(r#"{"id":"gpt-4o","providerID":"openai"}"#.to_string())
        );
        assert!((sessions[0].cost - 0.05).abs() < 0.001);
        assert_eq!(sessions[0].tokens_input, 1000);
        assert_eq!(sessions[0].tokens_output, 500);
        assert_eq!(sessions[0].tokens_reasoning, 100);
        assert_eq!(sessions[0].time_created, 1718000000000_i64);
        assert_eq!(sessions[0].time_updated, 1718003600000_i64);
    }

    #[test]
    fn insert_and_retrieve_messages() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test_opencode.db");
        let db = OpencodeDb::open(Some(db_path.to_str().unwrap())).unwrap();

        // Create tables with ACTUAL opencode schema
        db.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS message (\
                 id TEXT PRIMARY KEY, session_id TEXT, \
                 time_created INTEGER, time_updated INTEGER, data TEXT)",
                [],
            )
            .unwrap();

        db.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS part (\
                 id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT, \
                 time_created INTEGER, time_updated INTEGER, data TEXT)",
                [],
            )
            .unwrap();

        // Insert a user message (no tokens)
        db.conn
            .execute(
                "INSERT INTO message (id, session_id, time_created, time_updated, data) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    "msg-1",
                    "sess-1",
                    1718000000000_i64,
                    1718000000000_i64,
                    r#"{"role":"user","agent":"build"}"#
                ],
            )
            .unwrap();

        // Insert a part for msg-1
        db.conn
            .execute(
                "INSERT INTO part (id, message_id, session_id, time_created, time_updated, data) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    "prt-1",
                    "msg-1",
                    "sess-1",
                    1718000000000_i64,
                    1718000000000_i64,
                    r#"{"type":"text","text":"Hello"}"#
                ],
            )
            .unwrap();

        // Insert an assistant message (has tokens)
        db.conn
            .execute(
                "INSERT INTO message (id, session_id, time_created, time_updated, data) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    "msg-2",
                    "sess-1",
                    1718000060000_i64,
                    1718000060000_i64,
                    r#"{"role":"assistant","finish":"stop","tokens":{"total":50,"input":0,"output":50,"reasoning":0}}"#
                ],
            )
            .unwrap();

        // Insert part for msg-2
        db.conn
            .execute(
                "INSERT INTO part (id, message_id, session_id, time_created, time_updated, data) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    "prt-2",
                    "msg-2",
                    "sess-1",
                    1718000060000_i64,
                    1718000060000_i64,
                    r#"{"type":"text","text":"Hi there!"}"#
                ],
            )
            .unwrap();

        let messages = db.get_session_messages("sess-1").unwrap();
        assert_eq!(messages.len(), 2);

        assert_eq!(messages[0].message_id, "msg-1");
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].parts.len(), 1);
        assert_eq!(messages[0].parts[0].part_type, "text");
        assert_eq!(messages[0].parts[0].text, Some("Hello".to_string()));
        assert_eq!(messages[0].token_count, 0); // no tokens in user msg

        assert_eq!(messages[1].message_id, "msg-2");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].finish_reason, Some("stop".to_string()));
        assert_eq!(messages[1].token_count, 50);
        assert_eq!(messages[1].parts.len(), 1);
        assert_eq!(messages[1].parts[0].text, Some("Hi there!".to_string()));

        // Direct lookup
        let msg = db.get_session_message("sess-1", "msg-1").unwrap();
        assert!(msg.is_some());
        assert_eq!(msg.unwrap().message_id, "msg-1");

        // Unknown message
        let msg = db.get_session_message("sess-1", "msg-nonexistent").unwrap();
        assert!(msg.is_none());
    }

    #[test]
    fn list_sessions_with_directory_filter() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test_opencode.db");
        let db = OpencodeDb::open(Some(db_path.to_str().unwrap())).unwrap();

        db.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS session (\
                 id TEXT PRIMARY KEY, project_id TEXT, directory TEXT, \
                 title TEXT, agent TEXT, model TEXT, cost REAL, \
                 tokens_input INTEGER, tokens_output INTEGER, tokens_reasoning INTEGER, \
                 tokens_cache_read INTEGER, tokens_cache_write INTEGER, \
                 time_created INTEGER, time_updated INTEGER, data TEXT)",
                [],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO session \
                 (id, project_id, directory, title, agent, model, cost, \
                  tokens_input, tokens_output, tokens_reasoning, \
                  tokens_cache_read, tokens_cache_write, \
                  time_created, time_updated, data) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    "sess-1",
                    "proj-1",
                    "/home/user/project",
                    "Session 1",
                    "build",
                    r#"{"id":"gpt-4o"}"#,
                    0.0,
                    0_i64,
                    0_i64,
                    0_i64,
                    0_i64,
                    0_i64,
                    1718000000000_i64,
                    1718003600000_i64,
                    "{}"
                ],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO session \
                 (id, project_id, directory, title, agent, model, cost, \
                  tokens_input, tokens_output, tokens_reasoning, \
                  tokens_cache_read, tokens_cache_write, \
                  time_created, time_updated, data) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    "sess-2",
                    "proj-2",
                    "/tmp/other",
                    "Session 2",
                    "build",
                    r#"{"id":"claude"}"#,
                    0.0,
                    0_i64,
                    0_i64,
                    0_i64,
                    0_i64,
                    0_i64,
                    1718000000000_i64,
                    1718003600000_i64,
                    "{}"
                ],
            )
            .unwrap();

        // Filter by directory
        let sessions = db.list_sessions(Some("/home/user/project"), 50).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "sess-1");

        // No match
        let sessions = db.list_sessions(Some("/nonexistent"), 50).unwrap();
        assert!(sessions.is_empty());
    }

    #[test]
    fn list_sessions_respects_limit() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test_opencode.db");
        let db = OpencodeDb::open(Some(db_path.to_str().unwrap())).unwrap();

        db.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS session (\
                 id TEXT PRIMARY KEY, project_id TEXT, directory TEXT, \
                 title TEXT, agent TEXT, model TEXT, cost REAL, \
                 tokens_input INTEGER, tokens_output INTEGER, tokens_reasoning INTEGER, \
                 tokens_cache_read INTEGER, tokens_cache_write INTEGER, \
                 time_created INTEGER, time_updated INTEGER, data TEXT)",
                [],
            )
            .unwrap();

        for i in 0..5 {
            db.conn
                .execute(
                    "INSERT INTO session \
                     (id, project_id, directory, title, agent, model, cost, \
                      tokens_input, tokens_output, tokens_reasoning, \
                      tokens_cache_read, tokens_cache_write, \
                      time_created, time_updated, data) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                    params![
                        format!("sess-{}", i),
                        Option::<&str>::None,
                        Option::<&str>::None,
                        format!("Session {}", i),
                        "build",
                        r#"{"id":"gpt-4o"}"#,
                        0.0,
                        0_i64,
                        0_i64,
                        0_i64,
                        0_i64,
                        0_i64,
                        (1718000000 + i * 1000) as i64,
                        (1718003600 + i * 1000) as i64,
                        "{}"
                    ],
                )
                .unwrap();
        }

        let sessions = db.list_sessions(None, 3).unwrap();
        assert_eq!(sessions.len(), 3); // limited to 3
    }
}
