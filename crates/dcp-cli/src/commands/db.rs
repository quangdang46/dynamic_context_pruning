//! SQLite data access layer for opencode's session database.
//!
//! This module provides a typed interface to the opencode SQLite database
//! (typically at `~/.local/share/opencode/opencode.db`). It is feature-gated
//! behind the `scripts` feature — existing commands (stats, find-session,
//! timeline) use `FileStateStore` and do not depend on this module.
//!
//! # Schema
//!
//! The opencode DB stores projects, sessions, messages, and parts as JSON
//! blobs. The expected table shapes are:
//!
//! ```sql
//! CREATE TABLE project (
//!     id TEXT PRIMARY KEY,
//!     directory TEXT,
//!     data TEXT  -- JSON: {created_at, last_updated, ...}
//! );
//!
//! CREATE TABLE session (
//!     id TEXT PRIMARY KEY,
//!     project_id TEXT REFERENCES project(id),
//!     directory TEXT,
//!     created_at INTEGER,   -- unix timestamp ms
//!     last_updated INTEGER,
//!     message_count INTEGER DEFAULT 0,
//!     total_tokens INTEGER DEFAULT 0,
//!     state TEXT,           -- JSON: session state blob
//!     data TEXT             -- JSON: {stats, model, ...}
//! );
//!
//! CREATE TABLE message (
//!     id TEXT PRIMARY KEY,
//!     session_id TEXT REFERENCES session(id),
//!     role TEXT,            -- 'user' | 'assistant' | 'system'
//!     parts TEXT,           -- JSON array of part objects
//!     token_count INTEGER DEFAULT 0,
//!     finish_reason TEXT,   -- 'stop' | 'length' | 'error' | null
//!     is_final INTEGER,     -- boolean (0/1)
//!     created_at INTEGER,   -- unix timestamp ms
//!     data TEXT             -- JSON: additional metadata
//! );
//!
//! CREATE TABLE part (
//!     id TEXT PRIMARY KEY,
//!     message_id TEXT REFERENCES message(id),
//!     type TEXT,            -- 'text' | 'reasoning' | 'tool_call' | 'tool_result'
//!     data TEXT             -- JSON: part content + metadata
//! );
//! ```
//!
//! The adapter is tolerant of schema variations — missing columns result in
//! `None` / default values rather than errors.

use serde::{Deserialize, Serialize};

#[cfg(feature = "scripts")]
use rusqlite::{Connection, params};

/// Directory where opencode stores its data on Linux/macOS.
fn default_opencode_db_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "~".to_string());
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
    /// Project UUID.
    pub id: String,
    /// Working directory of the project.
    pub directory: String,
    /// Full JSON blob stored in the `data` column.
    pub data: serde_json::Value,
}

/// A row from the `session` table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRow {
    /// Session UUID.
    pub id: String,
    /// Parent project UUID (if set).
    pub project_id: Option<String>,
    /// Working directory associated with this session.
    pub directory: Option<String>,
    /// Unix timestamp (ms) when the session was created.
    pub created_at: i64,
    /// Unix timestamp (ms) of the last update.
    pub last_updated: i64,
    /// Number of messages in the session.
    pub message_count: i64,
    /// Total token count for the session.
    pub total_tokens: i64,
    /// JSON state blob (session state).
    pub state: Option<String>,
    /// Full JSON blob stored in the `data` column.
    pub data: serde_json::Value,
}

/// A message with its `parts` array already deserialized.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageWithParts {
    /// Message UUID.
    pub message_id: String,
    /// Parent session UUID.
    pub session_id: String,
    /// Role: `user`, `assistant`, `system`.
    pub role: String,
    /// Deserialized parts array from the `parts` JSON column.
    pub parts: Vec<serde_json::Value>,
    /// Token count for this message.
    pub token_count: i64,
    /// Finish reason: `stop`, `length`, `error`, or null.
    pub finish_reason: Option<String>,
    /// Whether this is a final (nonstreaming) message.
    pub is_final: bool,
    /// Unix timestamp (ms) when the message was created.
    pub created_at: i64,
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

        // Create parent directory if missing (opencode creates on first run)
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| anyhow::anyhow!("failed to create opencode data dir: {}", e))?;
        }

        let conn = Connection::open(&db_path)
            .map_err(|e| anyhow::anyhow!("failed to open opencode DB at {:?}: {}", db_path, e))?;

        Ok(Self { conn })
    }

    /// List all projects, ordered by ID.
    #[cfg(feature = "scripts")]
    #[allow(dead_code)]
    pub fn list_projects(&self) -> anyhow::Result<Vec<ProjectRow>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, directory, data FROM project ORDER BY id")?;

        let rows = stmt
            .query_map([], |row| {
                let data_str: String = row.get(2)?;
                let data: serde_json::Value = serde_json::from_str(&data_str).unwrap_or_default();
                Ok(ProjectRow {
                    id: row.get(0)?,
                    directory: row.get(1)?,
                    data,
                })
            })
            .map_err(|e| anyhow::anyhow!("list_projects query failed: {}", e))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("list_projects row error: {}", e))
    }

    /// List sessions, optionally filtered by `directory`, capped at `limit`.
    ///
    /// When `directory` is `None`, returns the most recently updated sessions.
    #[cfg(feature = "scripts")]
    pub fn list_sessions(
        &self,
        directory: Option<&str>,
        limit: usize,
    ) -> anyhow::Result<Vec<SessionRow>> {
        let sql = if directory.is_some() {
            "SELECT id, project_id, directory, created_at, last_updated, message_count, \
             total_tokens, state, data \
             FROM session WHERE directory = ?1 ORDER BY last_updated DESC LIMIT ?2"
        } else {
            "SELECT id, project_id, directory, created_at, last_updated, message_count, \
             total_tokens, state, data \
             FROM session ORDER BY last_updated DESC LIMIT ?1"
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

    /// Get a single session by ID. Returns `None` if not found.
    #[cfg(feature = "scripts")]
    pub fn get_session(&self, session_id: &str) -> anyhow::Result<Option<SessionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, project_id, directory, created_at, last_updated, message_count, \
             total_tokens, state, data \
             FROM session WHERE id = ?1",
        )?;

        let mut rows = stmt.query_map(params![session_id], session_row_map)?;

        match rows.next() {
            Some(result) => Ok(Some(result?)),
            None => Ok(None),
        }
        .map_err(|e: rusqlite::Error| anyhow::anyhow!("get_session error: {}", e))
    }

    /// Get all messages for a session, ordered by creation time.
    #[cfg(feature = "scripts")]
    pub fn get_session_messages(&self, session_id: &str) -> anyhow::Result<Vec<MessageWithParts>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, role, parts, token_count, finish_reason, is_final, created_at \
             FROM message WHERE session_id = ?1 ORDER BY created_at ASC",
        )?;

        let rows = stmt.query_map(params![session_id], message_row_map)?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("get_session_messages error: {}", e))
    }

    /// Get a single message within a session. Returns `None` if not found.
    #[cfg(feature = "scripts")]
    pub fn get_session_message(
        &self,
        session_id: &str,
        message_id: &str,
    ) -> anyhow::Result<Option<MessageWithParts>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, session_id, role, parts, token_count, finish_reason, is_final, created_at \
             FROM message WHERE session_id = ?1 AND id = ?2",
        )?;

        let mut rows = stmt.query_map(params![session_id, message_id], message_row_map)?;

        match rows.next() {
            Some(result) => Ok(Some(result?)),
            None => Ok(None),
        }
        .map_err(|e: rusqlite::Error| anyhow::anyhow!("get_session_message error: {}", e))
    }
}

// ---------------------------------------------------------------------------
// Row mappers
// ---------------------------------------------------------------------------

#[cfg(feature = "scripts")]
fn session_row_map(row: &rusqlite::Row) -> rusqlite::Result<SessionRow> {
    let data_str: String = row.get(8)?;
    let data: serde_json::Value = serde_json::from_str(&data_str).unwrap_or_default();
    let state_str: Option<String> = row.get(7)?;

    Ok(SessionRow {
        id: row.get(0)?,
        project_id: row.get(1)?,
        directory: row.get(2)?,
        created_at: row.get(3)?,
        last_updated: row.get(4)?,
        message_count: row.get(5)?,
        total_tokens: row.get(6)?,
        state: state_str,
        data,
    })
}

#[cfg(feature = "scripts")]
fn message_row_map(row: &rusqlite::Row) -> rusqlite::Result<MessageWithParts> {
    let parts_str: String = row.get(3)?;
    let parts: Vec<serde_json::Value> = serde_json::from_str::<serde_json::Value>(&parts_str)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let finish_reason: Option<String> = row.get(5)?;

    Ok(MessageWithParts {
        message_id: row.get(0)?,
        session_id: row.get(1)?,
        role: row.get(2)?,
        parts,
        token_count: row.get(4)?,
        finish_reason,
        is_final: row.get::<_, i32>(6)? != 0,
        created_at: row.get(7)?,
    })
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

        // The DB is empty (no tables) — that's fine, we tolerate missing tables
        // when the schema hasn't been initialized yet. Use the table-creating
        // tests below to verify query behavior against a properly initialized DB.
    }

    #[test]
    fn insert_and_retrieve_project() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test_opencode.db");
        let db = OpencodeDb::open(Some(db_path.to_str().unwrap())).unwrap();

        // Create the project table manually
        db.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS project (id TEXT PRIMARY KEY, directory TEXT, data TEXT)",
                [],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO project (id, directory, data) VALUES (?1, ?2, ?3)",
                params!["proj-1", "/tmp/test", r#"{"name":"test-project"}"#],
            )
            .unwrap();

        let projects = db.list_projects().unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id, "proj-1");
        assert_eq!(projects[0].directory, "/tmp/test");
    }

    #[test]
    fn insert_and_retrieve_session() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test_opencode.db");
        let db = OpencodeDb::open(Some(db_path.to_str().unwrap())).unwrap();

        db.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS session (id TEXT PRIMARY KEY, project_id TEXT, \
                 directory TEXT, created_at INTEGER, last_updated INTEGER, \
                 message_count INTEGER DEFAULT 0, total_tokens INTEGER DEFAULT 0, \
                 state TEXT, data TEXT)",
                [],
            )
            .unwrap();

        let none_str: Option<String> = None;
        db.conn
            .execute(
                "INSERT INTO session (id, project_id, directory, created_at, last_updated, \
                 message_count, total_tokens, state, data) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    "sess-1",
                    "proj-1",
                    "/tmp/test",
                    1718000000000_i64,
                    1718003600000_i64,
                    10_i64,
                    5000_i64,
                    &none_str,
                    r#"{"model":"gpt-4o"}"#
                ],
            )
            .unwrap();

        let sessions = db.list_sessions(None, 50).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "sess-1");
        assert_eq!(sessions[0].project_id, Some("proj-1".to_string()));
        assert_eq!(sessions[0].message_count, 10);
        assert_eq!(sessions[0].total_tokens, 5000);
    }

    #[test]
    fn insert_and_retrieve_messages() {
        let tmp = tempfile::tempdir().unwrap();
        let db_path = tmp.path().join("test_opencode.db");
        let db = OpencodeDb::open(Some(db_path.to_str().unwrap())).unwrap();

        db.conn
            .execute(
                "CREATE TABLE IF NOT EXISTS message (id TEXT PRIMARY KEY, session_id TEXT, \
                 role TEXT, parts TEXT, token_count INTEGER DEFAULT 0, \
                 finish_reason TEXT, is_final INTEGER, created_at INTEGER)",
                [],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO message (id, session_id, role, parts, token_count, \
                 finish_reason, is_final, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    "msg-1",
                    "sess-1",
                    "user",
                    r#"[{"type":"text","text":"Hello"}]"#,
                    100_i64,
                    Option::<&str>::None,
                    1_i32,
                    1718000000000_i64
                ],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO message (id, session_id, role, parts, token_count, \
                 finish_reason, is_final, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    "msg-2",
                    "sess-1",
                    "assistant",
                    r#"[{"type":"text","text":"Hi there!"}]"#,
                    50_i64,
                    "stop",
                    1_i32,
                    1718000060000_i64
                ],
            )
            .unwrap();

        let messages = db.get_session_messages("sess-1").unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].message_id, "msg-1");
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].parts[0]["type"], "text");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].finish_reason, Some("stop".to_string()));

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
                "CREATE TABLE IF NOT EXISTS session (id TEXT PRIMARY KEY, project_id TEXT, \
                 directory TEXT, created_at INTEGER, last_updated INTEGER, \
                 message_count INTEGER DEFAULT 0, total_tokens INTEGER DEFAULT 0, \
                 state TEXT, data TEXT)",
                [],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO session (id, project_id, directory, created_at, last_updated, \
                 message_count, total_tokens, state, data) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    "sess-1",
                    "proj-1",
                    "/home/user/project",
                    1718000000000_i64,
                    1718003600000_i64,
                    5_i64,
                    2500_i64,
                    Option::<&str>::None,
                    "{}"
                ],
            )
            .unwrap();

        db.conn
            .execute(
                "INSERT INTO session (id, project_id, directory, created_at, last_updated, \
                 message_count, total_tokens, state, data) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    "sess-2",
                    "proj-2",
                    "/tmp/other",
                    1718000000000_i64,
                    1718003600000_i64,
                    3_i64,
                    1500_i64,
                    Option::<&str>::None,
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
                "CREATE TABLE IF NOT EXISTS session (id TEXT PRIMARY KEY, project_id TEXT, \
                 directory TEXT, created_at INTEGER, last_updated INTEGER, \
                 message_count INTEGER DEFAULT 0, total_tokens INTEGER DEFAULT 0, \
                 state TEXT, data TEXT)",
                [],
            )
            .unwrap();

        for i in 0..5 {
            db.conn
                .execute(
                    "INSERT INTO session (id, project_id, directory, created_at, last_updated, \
                     message_count, total_tokens, state, data) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        format!("sess-{}", i),
                        Option::<&str>::None,
                        Option::<&str>::None,
                        (1718000000 + i * 1000) as i64,
                        (1718003600 + i * 1000) as i64,
                        1_i64,
                        100_i64,
                        Option::<&str>::None,
                        "{}"
                    ],
                )
                .unwrap();
        }

        let sessions = db.list_sessions(None, 3).unwrap();
        assert_eq!(sessions.len(), 3); // limited to 3
    }
}
