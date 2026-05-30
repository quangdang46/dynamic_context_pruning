#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]
//! `dcp-types` — canonical internal representation (IR) for the
//! `dynamic_context_pruning` library.
//!
//! This crate is the leaf of the workspace dependency graph. It defines the
//! format-agnostic value types every other `dcp-*` crate consumes:
//!
//! * [`Message`], [`Part`], [`Role`], [`ToolStatus`] — the canonical message
//!   shape (see SPEC.md §2).
//! * [`BlockId`], [`RunId`], [`MessageRef`] — the four identifier namespaces
//!   the library exposes to callers and to the model (SPEC.md §2.4).
//! * [`CompressionBlock`], [`CompressionMode`] — the unit of compressed
//!   conversation (SPEC.md §6.3).
//! * [`SessionState`] and its sub-structs — the in-memory bookkeeping
//!   (SPEC.md §9.1, PLAN.md §7.1).
//! * [`Stats`] and [`Telemetry`] — counters surfaced by the public facade.
//!
//! All types are `Serialize` + `Deserialize` so they can be persisted by
//! `dcp-storage` and round-tripped in tests.
//!
//! # Example
//!
//! ```rust
//! use dcp_types::{Message, Part, Role};
//!
//! let m = Message::user_text("u1", 0, "hello");
//! assert_eq!(m.role, Role::User);
//! assert_eq!(m.parts.len(), 1);
//! assert!(matches!(&m.parts[0], Part::Text(t) if t == "hello"));
//! ```

use std::collections::{HashMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

// ============================================================================
// Role
// ============================================================================

/// Producer of a [`Message`].
///
/// SPEC.md §2.3 — the library only emits `User` and `Assistant` messages in
/// transformed output; `System` content is handled through a separate
/// prompt-injection path.
///
/// Marked `#[non_exhaustive]` so additional roles (e.g. tool, function) can
/// be introduced in a backwards-compatible way.
///
/// # Example
///
/// ```rust
/// use dcp_types::Role;
/// assert_eq!(serde_json::to_string(&Role::User).unwrap(), "\"user\"");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Role {
    /// A user-authored message.
    User,
    /// An assistant-authored message.
    Assistant,
    /// A system message; emitted only by the host.
    System,
}

// ============================================================================
// ToolStatus
// ============================================================================

/// Lifecycle state of a tool call.
///
/// SPEC.md §2.2 — every `tool_result` part carries one of these values.
/// Marked `#[non_exhaustive]` to allow future states (e.g. `Cancelled`).
///
/// # Example
///
/// ```rust
/// use dcp_types::ToolStatus;
/// assert_eq!(serde_json::to_string(&ToolStatus::Completed).unwrap(), "\"completed\"");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum ToolStatus {
    /// The host has accepted the call but has not yet started running it.
    Pending,
    /// The tool is currently executing.
    Running,
    /// The tool finished successfully and `output` carries its result.
    Completed,
    /// The tool failed and `error` carries the diagnostic.
    Error,
}

// ============================================================================
// Part
// ============================================================================

/// A typed fragment of a [`Message`]'s payload.
///
/// SPEC.md §2.2 — a message is the ordered concatenation of its parts.
/// Marked `#[non_exhaustive]` so new variants (e.g. structured tool output,
/// audio) can be added without a major version bump.
///
/// Serialized as an externally-tagged enum: each variant becomes a JSON
/// object with a single key (`text`, `reasoning`, `tool_call`,
/// `tool_result`, `image`).
///
/// # Example
///
/// ```rust
/// use dcp_types::Part;
/// let p = Part::text("hello");
/// let json = serde_json::to_string(&p).unwrap();
/// assert_eq!(json, "{\"text\":\"hello\"}");
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Part {
    /// User-visible text body. May be empty.
    Text(String),
    /// Internal chain-of-thought, separate from `Text`.
    ///
    /// The library treats reasoning as content for token counting but never
    /// modifies it during pruning.
    Reasoning(String),
    /// An assistant-emitted request to execute a named tool.
    ToolCall {
        /// Call id, unique within the session, host-assigned.
        call_id: String,
        /// Name of the tool being invoked.
        tool: String,
        /// Arbitrary JSON input. The library normalizes it for signature
        /// computation (SPEC.md §4.4) but never mutates the original copy.
        input: JsonValue,
    },
    /// A user-emitted result of a previously seen [`Part::ToolCall`].
    ToolResult {
        /// Call id of the matching [`Part::ToolCall`].
        call_id: String,
        /// Lifecycle state at the time this result was produced.
        status: ToolStatus,
        /// Output payload when `status` is [`ToolStatus::Completed`].
        output: Option<String>,
        /// Diagnostic when `status` is [`ToolStatus::Error`].
        error: Option<String>,
    },
    /// An inline image. The library counts a fixed token cost per image and
    /// otherwise treats it as opaque.
    Image {
        /// IANA media type, e.g. `image/png`.
        media_type: String,
        /// Base64-encoded payload.
        data: String,
    },
}

impl Part {
    /// Construct a [`Part::Text`] from anything that can be turned into a
    /// `String`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::Part;
    /// let p = Part::text("hello");
    /// assert!(matches!(p, Part::Text(_)));
    /// ```
    pub fn text(s: impl Into<String>) -> Self {
        Part::Text(s.into())
    }

    /// Construct a [`Part::Reasoning`] from anything that can be turned into
    /// a `String`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::Part;
    /// let p = Part::reasoning("thinking…");
    /// assert!(matches!(p, Part::Reasoning(_)));
    /// ```
    pub fn reasoning(s: impl Into<String>) -> Self {
        Part::Reasoning(s.into())
    }

    /// Construct a [`Part::ToolCall`].
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::Part;
    /// let p = Part::tool_call("c1", "read_file", serde_json::json!({"path": "x"}));
    /// assert!(matches!(p, Part::ToolCall { .. }));
    /// ```
    pub fn tool_call(
        call_id: impl Into<String>,
        tool: impl Into<String>,
        input: JsonValue,
    ) -> Self {
        Part::ToolCall {
            call_id: call_id.into(),
            tool: tool.into(),
            input,
        }
    }

    /// Construct a [`Part::ToolResult`].
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::{Part, ToolStatus};
    /// let p = Part::tool_result("c1", ToolStatus::Completed, Some("ok".into()), None);
    /// assert!(matches!(p, Part::ToolResult { .. }));
    /// ```
    pub fn tool_result(
        call_id: impl Into<String>,
        status: ToolStatus,
        output: Option<String>,
        error: Option<String>,
    ) -> Self {
        Part::ToolResult {
            call_id: call_id.into(),
            status,
            output,
            error,
        }
    }

    /// Construct a [`Part::Image`].
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::Part;
    /// let p = Part::image("image/png", "QUFB");
    /// assert!(matches!(p, Part::Image { .. }));
    /// ```
    pub fn image(media_type: impl Into<String>, data: impl Into<String>) -> Self {
        Part::Image {
            media_type: media_type.into(),
            data: data.into(),
        }
    }

    /// True when this part is a [`Part::ToolCall`].
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::Part;
    /// assert!(Part::tool_call("c1", "t", serde_json::json!({})).is_tool_call());
    /// assert!(!Part::text("x").is_tool_call());
    /// ```
    pub fn is_tool_call(&self) -> bool {
        matches!(self, Part::ToolCall { .. })
    }

    /// True when this part is a [`Part::ToolResult`].
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::{Part, ToolStatus};
    /// assert!(Part::tool_result("c1", ToolStatus::Completed, None, None).is_tool_result());
    /// ```
    pub fn is_tool_result(&self) -> bool {
        matches!(self, Part::ToolResult { .. })
    }

    /// True when this part is a [`Part::Text`].
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::Part;
    /// assert!(Part::text("hello").is_text());
    /// assert!(!Part::reasoning("think").is_text());
    /// ```
    pub fn is_text(&self) -> bool {
        matches!(self, Part::Text(_))
    }
}

// ============================================================================
// Message
// ============================================================================

/// A single record in the conversation.
///
/// SPEC.md §2.1 — has exactly four logical fields. Messages are immutable
/// from the library's perspective; transformations always produce a new
/// sequence rather than mutating the input.
///
/// # Example
///
/// ```rust
/// use dcp_types::Message;
/// let m = Message::assistant_text("a1", 1234, "hi");
/// assert_eq!(m.id, "a1");
/// assert_eq!(m.time, 1234);
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// Opaque, host-assigned, unique within the session. Used as the
    /// dictionary key everywhere internal state references a message. The
    /// library does not interpret the format.
    pub id: String,
    /// Producer role.
    pub role: Role,
    /// Ordered list of parts. Validated to be non-empty before use.
    pub parts: Vec<Part>,
    /// Wall-clock millisecond timestamp. May be `0` if the host has no
    /// timestamp; never used for logic that affects pruning.
    pub time: i64,
    /// Marked ignored when the host has elided this message from the LLM's
    /// view. Ignored user messages are skipped by message-ref allocation
    /// and do not receive nudge tags. Defaults to `false`.
    #[serde(default)]
    pub ignored: bool,
}

impl Message {
    /// Construct a [`Message`] with arbitrary parts.
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::{Message, Part, Role};
    /// let m = Message::new("m1", Role::User, vec![Part::text("hi")], 0);
    /// assert_eq!(m.parts.len(), 1);
    /// ```
    pub fn new(id: impl Into<String>, role: Role, parts: Vec<Part>, time: i64) -> Self {
        Self {
            id: id.into(),
            role,
            parts,
            time,
            ignored: false,
        }
    }

    /// Construct a single-part [`Role::User`] message.
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::{Message, Role};
    /// let m = Message::user_text("u1", 0, "hello");
    /// assert_eq!(m.role, Role::User);
    /// ```
    pub fn user_text(id: impl Into<String>, time: i64, text: impl Into<String>) -> Self {
        Self::new(id, Role::User, vec![Part::text(text)], time)
    }

    /// Construct a single-part [`Role::Assistant`] message.
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::{Message, Role};
    /// let m = Message::assistant_text("a1", 0, "ack");
    /// assert_eq!(m.role, Role::Assistant);
    /// ```
    pub fn assistant_text(id: impl Into<String>, time: i64, text: impl Into<String>) -> Self {
        Self::new(id, Role::Assistant, vec![Part::text(text)], time)
    }

    /// Construct a single-part [`Role::System`] message.
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::{Message, Role};
    /// let m = Message::system_text("s1", 0, "you are…");
    /// assert_eq!(m.role, Role::System);
    /// ```
    pub fn system_text(id: impl Into<String>, time: i64, text: impl Into<String>) -> Self {
        Self::new(id, Role::System, vec![Part::text(text)], time)
    }
}

// ============================================================================
// BlockId / RunId
// ============================================================================

/// Block identifier — a positive `u32` allocated monotonically per session.
///
/// SPEC.md §2.4 — `BlockId(0)` is reserved as the "uninitialised" value the
/// allocator promotes to `1`; the library never exposes a block with id `0`.
/// Externally the id renders as `b<n>` (no padding).
///
/// # Example
///
/// ```rust
/// use dcp_types::BlockId;
/// let b = BlockId::new(7);
/// assert_eq!(b.value(), 7);
/// assert_eq!(b.reference(), "b7");
/// ```
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct BlockId(pub u32);

impl BlockId {
    /// Construct a [`BlockId`] from a raw `u32`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::BlockId;
    /// assert_eq!(BlockId::new(3).value(), 3);
    /// ```
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the raw underlying value.
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::BlockId;
    /// assert_eq!(BlockId::new(42).value(), 42);
    /// ```
    pub const fn value(self) -> u32 {
        self.0
    }

    /// Render as the canonical reference string `b<n>` (no padding).
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::BlockId;
    /// assert_eq!(BlockId::new(12).reference(), "b12");
    /// ```
    pub fn reference(self) -> String {
        format!("b{}", self.0)
    }
}

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "b{}", self.0)
    }
}

/// Run identifier — a positive `u32` allocated monotonically per session.
///
/// SPEC.md §2.4 — one run produces one or more [`CompressionBlock`]s. Like
/// [`BlockId`], `RunId(0)` is reserved as the uninitialised value.
///
/// # Example
///
/// ```rust
/// use dcp_types::RunId;
/// let r = RunId::new(5);
/// assert_eq!(r.value(), 5);
/// assert_eq!(r.to_string(), "r5");
/// ```
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct RunId(pub u32);

impl RunId {
    /// Construct a [`RunId`] from a raw `u32`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::RunId;
    /// assert_eq!(RunId::new(2).value(), 2);
    /// ```
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Return the raw underlying value.
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::RunId;
    /// assert_eq!(RunId::new(8).value(), 8);
    /// ```
    pub const fn value(self) -> u32 {
        self.0
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r{}", self.0)
    }
}

// ============================================================================
// MessageRef
// ============================================================================

/// Maximum legal value for a message reference index (SPEC.md §2.4).
const MESSAGE_REF_MAX: u32 = 9_999;

/// What a [`MessageRef`] points at.
///
/// Either a regular message (referenced as `m####`, zero-padded four-digit
/// decimal between `0001` and `9999`) or a compression block (referenced as
/// `b<n>`, no padding).
///
/// # Example
///
/// ```rust
/// use dcp_types::{BlockId, MessageRefKind};
/// let k = MessageRefKind::Block(BlockId::new(3));
/// assert!(matches!(k, MessageRefKind::Block(_)));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRefKind {
    /// A reference to a regular message, by its 1..=9999 index.
    Message(u32),
    /// A reference to a [`CompressionBlock`].
    Block(BlockId),
}

/// Library-assigned, session-stable reference exposed to the model.
///
/// SPEC.md §2.4 — references are permanent for the lifetime of a session
/// once allocated. [`MessageRef`] keeps both the parsed [`MessageRefKind`]
/// and the canonical raw string so callers can round-trip without
/// re-formatting.
///
/// Serialized as a plain string (`"m0001"` or `"b7"`); the kind is
/// recovered on deserialization.
///
/// # Example
///
/// ```rust
/// use dcp_types::MessageRef;
/// let r = MessageRef::message(42).unwrap();
/// assert_eq!(r.raw(), "m0042");
/// let parsed = MessageRef::parse("m0042").unwrap();
/// assert_eq!(parsed, r);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MessageRef {
    kind: MessageRefKind,
    raw: String,
}

/// Errors returned by [`MessageRef::parse`].
///
/// All variants identify a deterministic reason; matchers and tests can
/// rely on these tags. Marked `#[non_exhaustive]` to allow new error
/// classes without a major version bump.
///
/// # Example
///
/// ```rust
/// use dcp_types::{MessageRef, MessageRefParseError};
/// assert_eq!(MessageRef::parse(""), Err(MessageRefParseError::Empty));
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MessageRefParseError {
    /// Input was the empty string.
    Empty,
    /// Input did not start with `m` or `b`.
    InvalidPrefix,
    /// `m` ref did not have exactly four digits, or `b` ref had no digits.
    InvalidLength,
    /// A non-digit byte appeared after the prefix.
    InvalidDigit,
    /// `b` ref had a leading zero (e.g. `b01`).
    LeadingZero,
    /// Parsed value was `0` (not a legal allocator output).
    Zero,
    /// `m` ref value exceeded `MESSAGE_REF_MAX` (`9999`).
    OutOfRange,
}

impl fmt::Display for MessageRefParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Empty => "empty message reference",
            Self::InvalidPrefix => "message reference must start with 'm' or 'b'",
            Self::InvalidLength => "message reference has wrong digit count",
            Self::InvalidDigit => "message reference contains non-digit byte",
            Self::LeadingZero => "block reference has leading zero",
            Self::Zero => "message reference value must be positive",
            Self::OutOfRange => "m-reference value out of range (max 9999)",
        };
        f.write_str(s)
    }
}

impl std::error::Error for MessageRefParseError {}

impl MessageRef {
    /// Construct a message reference from a 1..=9999 index.
    ///
    /// Returns [`MessageRefParseError::Zero`] for `0` and
    /// [`MessageRefParseError::OutOfRange`] for values above `9999`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::MessageRef;
    /// let r = MessageRef::message(1).unwrap();
    /// assert_eq!(r.raw(), "m0001");
    /// ```
    pub fn message(index: u32) -> Result<Self, MessageRefParseError> {
        if index == 0 {
            return Err(MessageRefParseError::Zero);
        }
        if index > MESSAGE_REF_MAX {
            return Err(MessageRefParseError::OutOfRange);
        }
        Ok(Self {
            kind: MessageRefKind::Message(index),
            raw: format!("m{index:04}"),
        })
    }

    /// Construct a block reference.
    ///
    /// Returns [`MessageRefParseError::Zero`] when `block.value() == 0`,
    /// since `BlockId(0)` is reserved as the uninitialised allocator value.
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::{BlockId, MessageRef};
    /// let r = MessageRef::block(BlockId::new(3)).unwrap();
    /// assert_eq!(r.raw(), "b3");
    /// ```
    pub fn block(block: BlockId) -> Result<Self, MessageRefParseError> {
        if block.value() == 0 {
            return Err(MessageRefParseError::Zero);
        }
        Ok(Self {
            kind: MessageRefKind::Block(block),
            raw: format!("b{}", block.value()),
        })
    }

    /// Parse a reference string under strict rules.
    ///
    /// Strictness:
    /// * `m####` — exactly four ASCII digits, value in `1..=9999`.
    /// * `b<n>` — one or more ASCII digits, no leading zero, value in
    ///   `1..=u32::MAX`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::{MessageRef, MessageRefParseError};
    /// assert!(MessageRef::parse("m0001").is_ok());
    /// assert!(MessageRef::parse("b42").is_ok());
    /// assert_eq!(MessageRef::parse("m1"), Err(MessageRefParseError::InvalidLength));
    /// assert_eq!(MessageRef::parse("b01"), Err(MessageRefParseError::LeadingZero));
    /// ```
    pub fn parse(s: &str) -> Result<Self, MessageRefParseError> {
        if s.is_empty() {
            return Err(MessageRefParseError::Empty);
        }
        let bytes = s.as_bytes();
        match bytes[0] {
            b'm' => parse_message_ref(s),
            b'b' => parse_block_ref(s),
            _ => Err(MessageRefParseError::InvalidPrefix),
        }
    }

    /// Borrow the canonical raw string.
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::MessageRef;
    /// let r = MessageRef::message(7).unwrap();
    /// assert_eq!(r.raw(), "m0007");
    /// ```
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Borrow the parsed [`MessageRefKind`].
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::{MessageRef, MessageRefKind};
    /// let r = MessageRef::message(7).unwrap();
    /// assert!(matches!(r.kind(), MessageRefKind::Message(7)));
    /// ```
    pub fn kind(&self) -> MessageRefKind {
        self.kind
    }
}

fn parse_message_ref(s: &str) -> Result<MessageRef, MessageRefParseError> {
    debug_assert!(s.as_bytes().first() == Some(&b'm'));
    let digits = &s[1..];
    if digits.len() != 4 {
        return Err(MessageRefParseError::InvalidLength);
    }
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(MessageRefParseError::InvalidDigit);
    }
    // Safe: 4 ASCII digits fit in u32 (max 9999).
    let value: u32 = digits
        .parse()
        .map_err(|_| MessageRefParseError::InvalidDigit)?;
    if value == 0 {
        return Err(MessageRefParseError::Zero);
    }
    if value > MESSAGE_REF_MAX {
        return Err(MessageRefParseError::OutOfRange);
    }
    Ok(MessageRef {
        kind: MessageRefKind::Message(value),
        raw: format!("m{value:04}"),
    })
}

fn parse_block_ref(s: &str) -> Result<MessageRef, MessageRefParseError> {
    debug_assert!(s.as_bytes().first() == Some(&b'b'));
    let digits = &s[1..];
    if digits.is_empty() {
        return Err(MessageRefParseError::InvalidLength);
    }
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(MessageRefParseError::InvalidDigit);
    }
    if digits.len() > 1 && digits.as_bytes()[0] == b'0' {
        return Err(MessageRefParseError::LeadingZero);
    }
    let value: u32 = digits
        .parse()
        .map_err(|_| MessageRefParseError::OutOfRange)?;
    if value == 0 {
        return Err(MessageRefParseError::Zero);
    }
    Ok(MessageRef {
        kind: MessageRefKind::Block(BlockId(value)),
        raw: format!("b{value}"),
    })
}

impl fmt::Display for MessageRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.raw)
    }
}

impl Serialize for MessageRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.raw)
    }
}

impl<'de> Deserialize<'de> for MessageRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        MessageRef::parse(&s).map_err(serde::de::Error::custom)
    }
}

// ============================================================================
// CompressionMode + CompressionBlock
// ============================================================================

/// Mode in which a [`CompressionBlock`] was created.
///
/// SPEC.md §6 — `Range` covers a contiguous span of messages; `Message`
/// covers an individual non-contiguous message. Marked `#[non_exhaustive]`
/// to leave room for future modes.
///
/// # Example
///
/// ```rust
/// use dcp_types::CompressionMode;
/// assert_eq!(serde_json::to_string(&CompressionMode::Range).unwrap(), "\"range\"");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum CompressionMode {
    /// A contiguous range of messages, replaced by a single anchor.
    Range,
    /// A single targeted message, replaced in place.
    Message,
}

/// A unit of compressed conversation.
///
/// SPEC.md §6.3 — every block carries the fields below. Most fields are
/// immutable after commit; a small subset (marked `pub` here for ergonomic
/// access by downstream crates) may be mutated when the block is consumed
/// by a parent block.
///
/// # Example
///
/// ```rust
/// use dcp_types::{BlockId, CompressionBlock, CompressionMode, RunId};
/// let b = CompressionBlock::new(
///     BlockId::new(1),
///     RunId::new(1),
///     CompressionMode::Range,
///     "topic",
///     "summary",
///     "m0001",
///     "m0003",
///     "raw-anchor",
///     "raw-compress",
/// );
/// assert!(b.active);
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompressionBlock {
    /// Allocated by `allocate_block_id`.
    pub block_id: BlockId,
    /// Allocated once per compress call.
    pub run_id: RunId,
    /// `Range` or `Message`.
    pub mode: CompressionMode,
    /// Batch-level topic from the compress call.
    pub topic: String,
    /// Same as `topic` for range mode; the per-entry topic for message mode.
    pub batch_topic: Option<String>,
    /// Wrapped summary (placeholder-expanded, with protected appendices).
    pub summary: String,
    /// Original `startId` (range mode) or `messageId` (message mode), in
    /// reference form.
    pub start_id: String,
    /// Original `endId` (= `messageId` in message mode), in reference form.
    pub end_id: String,
    /// Raw id of the anchor message.
    pub anchor_message_id: String,
    /// Raw id of the assistant message that issued the compress call.
    pub compress_message_id: String,
    /// Tool call id of the compress invocation, if present.
    pub compress_call_id: Option<String>,
    /// All block ids whose summaries were folded into this block (consumed
    /// plus their transitive includes).
    pub included_block_ids: Vec<BlockId>,
    /// Block ids that were directly active at compression time and are now
    /// deactivated by this block.
    pub consumed_block_ids: Vec<BlockId>,
    /// Block ids that have consumed this block (mutated when superseded).
    pub parent_block_ids: Vec<BlockId>,
    /// Raw message ids covered directly (excluding messages already inside
    /// consumed blocks).
    pub direct_message_ids: Vec<String>,
    /// Tool call ids referenced by `direct_message_ids`.
    pub direct_tool_ids: Vec<String>,
    /// Union of `direct_message_ids` and the consumed blocks'
    /// `effective_message_ids`.
    pub effective_message_ids: Vec<String>,
    /// Union of `direct_tool_ids` and the consumed blocks'
    /// `effective_tool_ids`.
    pub effective_tool_ids: Vec<String>,
    /// Sum of token counts over the verbatim content this block replaces
    /// (best-effort estimate at commit time).
    pub compressed_tokens: u64,
    /// Token count of the wrapped summary string.
    pub summary_tokens: u64,
    /// Time spent assembling the block (host-clock, informational).
    pub duration_ms: u64,
    /// True while the block is the current representation of its anchor;
    /// false after a parent block consumes it.
    pub active: bool,
    /// True if the user explicitly decompressed via the slash command.
    pub deactivated_by_user: bool,
    /// Wall-clock millisecond timestamp at commit.
    pub created_at: i64,
    /// Set when `active` becomes `false`.
    pub deactivated_at: Option<i64>,
    /// Set when consumed by a parent block; otherwise `None`.
    pub deactivated_by_block_id: Option<BlockId>,
}

impl CompressionBlock {
    /// Construct a freshly-committed [`CompressionBlock`] with all derived
    /// lists empty and `active = true`.
    ///
    /// Most fields a downstream crate sets later are exposed as `pub`, so
    /// the typical pattern is to call this constructor and then populate
    /// `included_block_ids`, `direct_message_ids`, etc.
    ///
    /// # Example
    ///
    /// ```rust
    /// use dcp_types::{BlockId, CompressionBlock, CompressionMode, RunId};
    /// let b = CompressionBlock::new(
    ///     BlockId::new(1), RunId::new(1), CompressionMode::Range,
    ///     "t", "s", "m0001", "m0002", "raw1", "raw2",
    /// );
    /// assert_eq!(b.block_id, BlockId::new(1));
    /// ```
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        block_id: BlockId,
        run_id: RunId,
        mode: CompressionMode,
        topic: impl Into<String>,
        summary: impl Into<String>,
        start_id: impl Into<String>,
        end_id: impl Into<String>,
        anchor_message_id: impl Into<String>,
        compress_message_id: impl Into<String>,
    ) -> Self {
        Self {
            block_id,
            run_id,
            mode,
            topic: topic.into(),
            batch_topic: None,
            summary: summary.into(),
            start_id: start_id.into(),
            end_id: end_id.into(),
            anchor_message_id: anchor_message_id.into(),
            compress_message_id: compress_message_id.into(),
            compress_call_id: None,
            included_block_ids: Vec::new(),
            consumed_block_ids: Vec::new(),
            parent_block_ids: Vec::new(),
            direct_message_ids: Vec::new(),
            direct_tool_ids: Vec::new(),
            effective_message_ids: Vec::new(),
            effective_tool_ids: Vec::new(),
            compressed_tokens: 0,
            summary_tokens: 0,
            duration_ms: 0,
            active: true,
            deactivated_by_user: false,
            created_at: 0,
            deactivated_at: None,
            deactivated_by_block_id: None,
        }
    }
}

// ============================================================================
// Stats
// ============================================================================

/// Persisted counters surfaced by `ContextPruner::stats`.
///
/// SPEC.md §9.1 — every counter increases monotonically over the life of a
/// session and survives reload via [`SessionState`] persistence.
///
/// # Example
///
/// ```rust
/// use dcp_types::Stats;
/// let s = Stats::default();
/// assert_eq!(s.total_prune_tokens, 0);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stats {
    /// Sum of token counts for every part removed by any prune strategy.
    pub total_prune_tokens: u64,
    /// Number of tool calls dropped by the deduplicate strategy.
    pub dedup_pruned: u32,
    /// Number of tool calls dropped by the purge-errors strategy.
    pub purge_errors_pruned: u32,
    /// Number of tool calls dropped by the stale-file-reads strategy.
    pub stale_file_reads_pruned: u32,
    /// Number of `compress` tool invocations that ran to completion.
    pub compress_runs: u32,
    /// Number of [`CompressionBlock`]s actually committed.
    pub compress_blocks_committed: u32,
    /// Number of attempted compressions whose summary was larger than the
    /// raw range it would replace.
    pub compress_oversized: u32,
    /// Number of attempted compressions deemed worth keeping.
    pub compress_useful: u32,
    /// Number of host-side compaction events the library detected.
    pub compactions_observed: u32,
    /// Number of times the cache had to be invalidated.
    pub cache_bust_events: u32,
    /// Number of `tool_result` parts whose `call_id` could not be paired
    /// with a prior `tool_call`.
    pub orphan_tool_results: u32,
    /// Number of input messages dropped during validation.
    pub dropped_invalid: u32,
    /// Number of attempted [`ToolStatus`] transitions that violated the
    /// state machine.
    pub invalid_status_transitions: u32,
    /// Number of times JSON normalization had to clamp recursion depth.
    pub normalize_depth_clamped: u32,
    /// Number of paths with embedded null bytes that had to be sanitised.
    pub path_null_byte_stripped: u32,
    /// Number of failures from the storage backend during save.
    pub storage_save_failed: u32,
    /// Number of corrupted persisted documents observed at load time.
    pub persisted_corruption: u32,
}

// ============================================================================
// Telemetry
// ============================================================================

/// Lightweight, in-process event counters distinct from [`Stats`].
///
/// PLAN.md §4.2 — surfaced by `ContextPruner::telemetry`. Whereas [`Stats`]
/// captures session-lifetime totals that survive persistence, [`Telemetry`]
/// is intended for operational observability of the *current* process and
/// is not persisted.
///
/// # Example
///
/// ```rust
/// use dcp_types::Telemetry;
/// let mut t = Telemetry::default();
/// t.transforms_total += 1;
/// assert_eq!(t.transforms_total, 1);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Telemetry {
    /// Number of `transform_messages` invocations.
    pub transforms_total: u64,
    /// Number of times the apply phase actually ran.
    pub apply_phase_runs: u64,
    /// Number of times the apply phase was skipped because of cache
    /// stability gating.
    pub apply_phase_skipped: u64,
    /// Number of pending-prune snapshots that were eventually flushed.
    pub pending_prune_flushed: u64,
    /// Number of context-limit nudges emitted.
    pub nudges_context_limit: u64,
    /// Number of turn nudges emitted.
    pub nudges_turn: u64,
    /// Number of iteration nudges emitted.
    pub nudges_iteration: u64,
    /// Number of compress tool invocations the host accepted.
    pub compress_invocations: u64,
    /// Cache-bust events observed by the cache accountant.
    pub cache_bust_events: u64,
    /// Per-strategy counters keyed by strategy name (`"deduplicate"`,
    /// `"purge_errors"`, `"stale_file_reads"`, …).
    pub strategy_runs: HashMap<String, u64>,
}

// ============================================================================
// SessionState and supporting structs
// ============================================================================

/// Manual-mode state.
///
/// SPEC.md §5 — when `enabled`, automatic strategies do not write decisions
/// unless the configuration's `automatic_strategies` flag is also set.
///
/// # Example
///
/// ```rust
/// use dcp_types::ManualMode;
/// let m = ManualMode::default();
/// assert!(!m.enabled);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualMode {
    /// True when manual mode is active.
    pub enabled: bool,
}

/// Whether the host has granted permission to run the `compress` tool.
///
/// SPEC.md §6 — the host can either pre-approve compression, ask each time,
/// or deny it outright. Default is [`CompressPermission::Allow`].
///
/// # Example
///
/// ```rust
/// use dcp_types::CompressPermission;
/// assert_eq!(CompressPermission::default(), CompressPermission::Allow);
/// ```
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompressPermission {
    /// Compression may run without further consent.
    #[default]
    Allow,
    /// The host must be prompted before each compression.
    Ask,
    /// Compression is not allowed.
    Deny,
}

/// Pending host-driven manual trigger.
///
/// SPEC.md §5.4 — when the host queues an explicit prune/compress request,
/// the library records the trigger so it can be applied at the next
/// `transform_messages` invocation.
///
/// # Example
///
/// ```rust
/// use dcp_types::PendingManualTrigger;
/// let t = PendingManualTrigger::default();
/// assert!(t.reason.is_empty());
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingManualTrigger {
    /// Human-readable reason recorded by the host.
    pub reason: String,
    /// True when the host wants the apply phase to run regardless of cache
    /// stability gating.
    pub force_apply: bool,
}

/// Snapshot of prune decisions waiting to reach the outgoing message stream.
///
/// SPEC.md §7.2 — strategies always run, but applying their decisions can
/// be deferred under cache stability gating. A `PendingPrune` records what
/// has been accumulated since the last apply.
///
/// # Example
///
/// ```rust
/// use dcp_types::PendingPrune;
/// let p = PendingPrune::default();
/// assert!(p.tool_ids.is_empty());
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingPrune {
    /// Tool call ids newly added to `state.prune.tools` since the last
    /// apply.
    pub tool_ids: Vec<String>,
    /// Cumulative tokens that would be saved if the pending decisions were
    /// applied right now.
    pub cumulative_tokens: u64,
    /// `state.current_turn` at the moment the snapshot was created.
    pub accumulated_at_turn: u32,
}

/// Per-tool bookkeeping entry.
///
/// SPEC.md §9.1 — populated by the tool tracker (SPEC.md §4) and consumed
/// by every prune strategy.
///
/// # Example
///
/// ```rust
/// use dcp_types::{ToolParameterEntry, ToolStatus};
/// let mut e = ToolParameterEntry::default();
/// e.tool = "read_file".into();
/// e.status = Some(ToolStatus::Completed);
/// assert_eq!(e.tool, "read_file");
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolParameterEntry {
    /// Tool name, e.g. `"read_file"`.
    pub tool: String,
    /// Canonical signature string used for deduplication.
    pub signature: String,
    /// Last observed lifecycle state.
    pub status: Option<ToolStatus>,
    /// `state.current_turn` at the time this call was registered.
    pub turn: u32,
    /// Raw id of the assistant message that emitted the call.
    pub message_id: String,
    /// Raw id of the user message that carried the result, if any.
    pub result_message_id: Option<String>,
    /// File paths referenced by the call's input (for path-based protection).
    pub paths: Vec<String>,
    /// Token count attributed to the call's input/result.
    pub token_count: Option<u64>,
}

/// Mapping between host-assigned raw ids and library-allocated `m####`
/// references.
///
/// SPEC.md §2.4 — references are stable for the lifetime of a session; the
/// allocator stores both directions plus the next free index.
///
/// # Example
///
/// ```rust
/// use dcp_types::MessageIdState;
/// let s = MessageIdState::default();
/// assert_eq!(s.next_ref, 0);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageIdState {
    /// Lookup from raw id to canonical reference.
    pub by_raw_id: HashMap<String, String>,
    /// Lookup from canonical reference to raw id.
    pub by_ref: HashMap<String, String>,
    /// Next free index. `0` means "not yet initialised", per SPEC.md §2.4.
    pub next_ref: u32,
}

/// Per-message prune entry.
///
/// SPEC.md §5 — used when a strategy rewrites a single message rather than
/// dropping it. Phase 1 declares the shape; later phases populate it.
///
/// # Example
///
/// ```rust
/// use dcp_types::PrunedMessageEntry;
/// let p = PrunedMessageEntry::default();
/// assert!(p.replacement_text.is_empty());
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrunedMessageEntry {
    /// Replacement text rendered into the outgoing message.
    pub replacement_text: String,
    /// Token count of the original content this entry replaces.
    pub tokens_saved: u64,
}

/// Block-level prune state.
///
/// SPEC.md §6.3 / PLAN.md §7.1 — the `messages` half of [`Prune`].
///
/// # Example
///
/// ```rust
/// use dcp_types::PruneMessagesState;
/// let s = PruneMessagesState::default();
/// assert!(s.blocks_by_id.is_empty());
/// ```
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PruneMessagesState {
    /// Per-message prune decisions, keyed by raw message id.
    pub by_message_id: HashMap<String, PrunedMessageEntry>,
    /// Every block created during the session, keyed by id.
    pub blocks_by_id: HashMap<BlockId, CompressionBlock>,
    /// Block ids currently representing their anchor message.
    pub active_block_ids: HashSet<BlockId>,
    /// Lookup from anchor message raw id to the active block at that anchor.
    pub active_by_anchor_message_id: HashMap<String, BlockId>,
    /// Allocator: next [`BlockId`] to hand out. `BlockId(0)` is uninitialised.
    pub next_block_id: BlockId,
    /// Allocator: next [`RunId`] to hand out. `RunId(0)` is uninitialised.
    pub next_run_id: RunId,
    /// When set, suppresses nudges and compress suggestions targeting
    /// references at-or-before this point.
    pub frontier_message_ref: Option<String>,
}

/// Top-level prune state — tool decisions plus block-level decisions.
///
/// PLAN.md §7.1.
///
/// # Example
///
/// ```rust
/// use dcp_types::Prune;
/// let p = Prune::default();
/// assert!(p.tools.is_empty());
/// ```
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Prune {
    /// Tool calls marked for pruning, mapped to tokens saved.
    pub tools: HashMap<String, u64>,
    /// Block-level prune state.
    pub messages: PruneMessagesState,
}

/// Nudge scheduling state.
///
/// SPEC.md §8 — counters and the set of (user, assistant) pairs already
/// nudged.
///
/// # Example
///
/// ```rust
/// use dcp_types::Nudges;
/// let n = Nudges::default();
/// assert_eq!(n.context_limit_counter, 0);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Nudges {
    /// Counter that paces context-limit nudges (SPEC.md §8.1).
    pub context_limit_counter: u32,
    /// Counter that paces iteration nudges (SPEC.md §8.3).
    pub iteration_counter: u32,
    /// Pairs already turn-nudged, stored as `(user_id, assistant_id)`.
    pub turn_nudged_pairs: HashSet<(String, String)>,
    /// Kind of the most recently injected nudge, if any.
    pub last_nudge_kind: Option<String>,
}

/// Telemetry-only timing accumulator for compression runs.
///
/// SPEC.md §6 — informational only; never affects pruning decisions.
///
/// # Example
///
/// ```rust
/// use dcp_types::CompressionTimingState;
/// let t = CompressionTimingState::default();
/// assert_eq!(t.total_compress_ms, 0);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompressionTimingState {
    /// Sum of `duration_ms` across every compress run in the session.
    pub total_compress_ms: u64,
    /// Number of compress runs sampled.
    pub samples: u32,
}

/// Library bookkeeping for a single conversation.
///
/// SPEC.md §9.1 / PLAN.md §7.1 — the in-memory shape every `dcp-*` crate
/// reads and writes through. `dcp-storage` persists a subset of these
/// fields per the persistence schema (SPEC.md §9.1).
///
/// # Example
///
/// ```rust
/// use dcp_types::SessionState;
/// let s = SessionState::default();
/// assert_eq!(s.current_turn, 0);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionState {
    /// Host-assigned session identifier, when known.
    pub session_id: Option<String>,
    /// True when this state belongs to a sub-agent run.
    pub is_subagent: bool,
    /// Manual-mode flags.
    pub manual_mode: ManualMode,
    /// Whether the compress tool is permitted.
    pub compress_permission: CompressPermission,
    /// Pending host-driven trigger, applied at the next transform.
    pub pending_manual_trigger: Option<PendingManualTrigger>,
    /// Tool and block prune state.
    pub prune: Prune,
    /// Nudge state.
    pub nudges: Nudges,
    /// Persisted counters.
    pub stats: Stats,
    /// Compression timing telemetry.
    pub compression_timing: CompressionTimingState,
    /// Per-tool bookkeeping, keyed by tool call id.
    pub tool_parameters: HashMap<String, ToolParameterEntry>,
    /// Cached results from sub-agent invocations, keyed by signature.
    pub subagent_result_cache: HashMap<String, String>,
    /// Tool call ids in the order they were observed.
    pub tool_id_list: Vec<String>,
    /// Message-reference allocator state.
    pub message_ids: MessageIdState,
    /// Wall-clock millisecond timestamp of the most recent compaction event.
    pub last_compaction: i64,
    /// Monotonically increasing turn counter (SPEC.md §3.2).
    pub current_turn: u32,
    /// Optional model context limit, in tokens.
    pub model_context_limit: Option<u64>,
    /// Optional system-prompt token count.
    pub system_prompt_tokens: Option<u64>,
    /// Whether the most recent message in the input was an assistant text
    /// message — drives the `agent_message` cache stability mode.
    pub last_message_was_assistant_text: bool,
    /// Snapshot of decisions waiting to reach the outgoing stream.
    pub pending_prune: Option<PendingPrune>,
    /// Turn at which the most recent apply phase ran.
    pub last_apply_turn: Option<u32>,
    /// True when the host has called `force_apply()` and the next transform
    /// must apply pending decisions regardless of gating.
    pub force_apply_requested: bool,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ----- Role -----

    #[test]
    fn role_serde_roundtrip() {
        for r in [Role::User, Role::Assistant, Role::System] {
            let s = serde_json::to_string(&r).unwrap();
            let back: Role = serde_json::from_str(&s).unwrap();
            assert_eq!(r, back);
        }
        assert_eq!(
            serde_json::to_string(&Role::Assistant).unwrap(),
            "\"assistant\""
        );
    }

    // ----- ToolStatus -----

    #[test]
    fn tool_status_serde_roundtrip() {
        for s in [
            ToolStatus::Pending,
            ToolStatus::Running,
            ToolStatus::Completed,
            ToolStatus::Error,
        ] {
            let json = serde_json::to_string(&s).unwrap();
            let back: ToolStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
        assert_eq!(
            serde_json::to_string(&ToolStatus::Pending).unwrap(),
            "\"pending\""
        );
    }

    // ----- Part -----

    #[test]
    fn part_text_serde_roundtrip() {
        let p = Part::text("hello");
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, r#"{"text":"hello"}"#);
        let back: Part = serde_json::from_str(&s).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn part_reasoning_serde_roundtrip() {
        let p = Part::reasoning("ponder");
        let back: Part = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn part_tool_call_serde_roundtrip() {
        let p = Part::tool_call("c1", "read_file", json!({"path": "x"}));
        let back: Part = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn part_tool_result_serde_roundtrip() {
        let p = Part::tool_result("c1", ToolStatus::Error, None, Some("boom".into()));
        let back: Part = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn part_image_serde_roundtrip() {
        let p = Part::image("image/png", "AAAA");
        let back: Part = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }

    // ----- Message -----

    #[test]
    fn message_serde_roundtrip() {
        let m = Message::new(
            "m-1",
            Role::Assistant,
            vec![
                Part::text("hello"),
                Part::tool_call("c1", "echo", json!({"v": 1})),
            ],
            42,
        );
        let s = serde_json::to_string(&m).unwrap();
        let back: Message = serde_json::from_str(&s).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn message_constructors_set_role() {
        assert_eq!(Message::user_text("u", 0, "x").role, Role::User);
        assert_eq!(Message::assistant_text("a", 0, "x").role, Role::Assistant);
        assert_eq!(Message::system_text("s", 0, "x").role, Role::System);
    }

    // ----- BlockId / RunId -----

    #[test]
    fn block_id_serde_transparent() {
        let b = BlockId::new(7);
        let s = serde_json::to_string(&b).unwrap();
        assert_eq!(s, "7");
        let back: BlockId = serde_json::from_str(&s).unwrap();
        assert_eq!(b, back);
    }

    #[test]
    fn block_id_display_and_reference() {
        assert_eq!(BlockId::new(5).to_string(), "b5");
        assert_eq!(BlockId::new(123).reference(), "b123");
    }

    #[test]
    fn block_id_monotonicity() {
        // Simulate the allocator from SPEC.md §2.4.
        let mut next = BlockId::new(1);
        let mut allocated = Vec::new();
        for _ in 0..10 {
            let id = next;
            allocated.push(id);
            next = BlockId::new(next.value() + 1);
        }
        // Strictly increasing, no duplicates.
        for w in allocated.windows(2) {
            assert!(w[0] < w[1]);
        }
        let unique: HashSet<_> = allocated.iter().copied().collect();
        assert_eq!(unique.len(), allocated.len());
        // First id is 1, last is 10.
        assert_eq!(allocated.first().copied(), Some(BlockId::new(1)));
        assert_eq!(allocated.last().copied(), Some(BlockId::new(10)));
    }

    #[test]
    fn run_id_serde_and_display() {
        let r = RunId::new(3);
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(s, "3");
        let back: RunId = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);
        assert_eq!(r.to_string(), "r3");
    }

    // ----- MessageRef -----

    #[test]
    fn message_ref_format_is_zero_padded() {
        assert_eq!(MessageRef::message(1).unwrap().raw(), "m0001");
        assert_eq!(MessageRef::message(42).unwrap().raw(), "m0042");
        assert_eq!(MessageRef::message(9999).unwrap().raw(), "m9999");
    }

    #[test]
    fn block_ref_format_has_no_padding() {
        assert_eq!(MessageRef::block(BlockId::new(1)).unwrap().raw(), "b1");
        assert_eq!(MessageRef::block(BlockId::new(123)).unwrap().raw(), "b123");
    }

    #[test]
    fn message_ref_constructor_rejects_invalid() {
        assert_eq!(MessageRef::message(0), Err(MessageRefParseError::Zero));
        assert_eq!(
            MessageRef::message(10_000),
            Err(MessageRefParseError::OutOfRange)
        );
        assert_eq!(
            MessageRef::block(BlockId::new(0)),
            Err(MessageRefParseError::Zero)
        );
    }

    #[test]
    fn message_ref_parse_strict() {
        // Valid.
        let r = MessageRef::parse("m0042").unwrap();
        assert_eq!(r.raw(), "m0042");
        assert_eq!(r.kind(), MessageRefKind::Message(42));

        let b = MessageRef::parse("b7").unwrap();
        assert_eq!(b.raw(), "b7");
        assert_eq!(b.kind(), MessageRefKind::Block(BlockId::new(7)));

        // Invalid forms.
        assert_eq!(MessageRef::parse(""), Err(MessageRefParseError::Empty));
        assert_eq!(
            MessageRef::parse("x0001"),
            Err(MessageRefParseError::InvalidPrefix)
        );
        assert_eq!(
            MessageRef::parse("m1"),
            Err(MessageRefParseError::InvalidLength)
        );
        assert_eq!(
            MessageRef::parse("m00001"),
            Err(MessageRefParseError::InvalidLength)
        );
        assert_eq!(
            MessageRef::parse("m000a"),
            Err(MessageRefParseError::InvalidDigit)
        );
        assert_eq!(MessageRef::parse("m0000"), Err(MessageRefParseError::Zero));
        assert_eq!(
            MessageRef::parse("b"),
            Err(MessageRefParseError::InvalidLength)
        );
        assert_eq!(MessageRef::parse("b0"), Err(MessageRefParseError::Zero));
        assert_eq!(
            MessageRef::parse("b01"),
            Err(MessageRefParseError::LeadingZero)
        );
        assert_eq!(
            MessageRef::parse("bx"),
            Err(MessageRefParseError::InvalidDigit)
        );
    }

    #[test]
    fn message_ref_round_trip_format_parse() {
        for n in [1u32, 42, 9999] {
            let r = MessageRef::message(n).unwrap();
            let parsed = MessageRef::parse(r.raw()).unwrap();
            assert_eq!(parsed, r);
        }
        for n in [1u32, 7, 100, u32::MAX] {
            let r = MessageRef::block(BlockId::new(n)).unwrap();
            let parsed = MessageRef::parse(r.raw()).unwrap();
            assert_eq!(parsed, r);
        }
    }

    #[test]
    fn message_ref_serde_roundtrip() {
        let r = MessageRef::message(1234).unwrap();
        let s = serde_json::to_string(&r).unwrap();
        assert_eq!(s, "\"m1234\"");
        let back: MessageRef = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);

        let b = MessageRef::block(BlockId::new(5)).unwrap();
        let s = serde_json::to_string(&b).unwrap();
        assert_eq!(s, "\"b5\"");
        let back: MessageRef = serde_json::from_str(&s).unwrap();
        assert_eq!(b, back);
    }

    #[test]
    fn message_ref_serde_rejects_invalid() {
        let bad: Result<MessageRef, _> = serde_json::from_str("\"m1\"");
        assert!(bad.is_err());
    }

    // ----- CompressionMode -----

    #[test]
    fn compression_mode_serde_roundtrip() {
        for m in [CompressionMode::Range, CompressionMode::Message] {
            let s = serde_json::to_string(&m).unwrap();
            let back: CompressionMode = serde_json::from_str(&s).unwrap();
            assert_eq!(m, back);
        }
    }

    // ----- CompressionBlock -----

    #[test]
    fn compression_block_serde_roundtrip() {
        let mut b = CompressionBlock::new(
            BlockId::new(1),
            RunId::new(1),
            CompressionMode::Range,
            "topic",
            "summary",
            "m0001",
            "m0003",
            "raw1",
            "raw2",
        );
        b.direct_message_ids = vec!["raw1".into(), "raw2".into()];
        b.included_block_ids = vec![BlockId::new(0)];
        b.consumed_block_ids = vec![];
        b.created_at = 1_700_000_000_000;
        b.compressed_tokens = 500;
        b.summary_tokens = 80;
        let s = serde_json::to_string(&b).unwrap();
        let back: CompressionBlock = serde_json::from_str(&s).unwrap();
        assert_eq!(b, back);
    }

    // ----- Stats -----

    #[test]
    fn stats_serde_roundtrip() {
        let s = Stats {
            total_prune_tokens: 1234,
            dedup_pruned: 7,
            compress_blocks_committed: 3,
            ..Stats::default()
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: Stats = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    // ----- Telemetry -----

    #[test]
    fn telemetry_serde_roundtrip() {
        let mut t = Telemetry {
            transforms_total: 9,
            ..Telemetry::default()
        };
        t.strategy_runs.insert("deduplicate".into(), 4);
        let s = serde_json::to_string(&t).unwrap();
        let back: Telemetry = serde_json::from_str(&s).unwrap();
        assert_eq!(t, back);
    }

    // ----- ManualMode / CompressPermission / PendingManualTrigger -----

    #[test]
    fn manual_mode_serde_roundtrip() {
        let m = ManualMode { enabled: true };
        let back: ManualMode = serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn compress_permission_serde_roundtrip() {
        for p in [
            CompressPermission::Allow,
            CompressPermission::Ask,
            CompressPermission::Deny,
        ] {
            let s = serde_json::to_string(&p).unwrap();
            let back: CompressPermission = serde_json::from_str(&s).unwrap();
            assert_eq!(p, back);
        }
        assert_eq!(
            serde_json::to_string(&CompressPermission::Allow).unwrap(),
            "\"allow\""
        );
    }

    #[test]
    fn pending_manual_trigger_serde_roundtrip() {
        let p = PendingManualTrigger {
            reason: "/dcp force".into(),
            force_apply: true,
        };
        let back: PendingManualTrigger =
            serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }

    // ----- PendingPrune / ToolParameterEntry / MessageIdState -----

    #[test]
    fn pending_prune_serde_roundtrip() {
        let p = PendingPrune {
            tool_ids: vec!["c1".into(), "c2".into()],
            cumulative_tokens: 256,
            accumulated_at_turn: 4,
        };
        let back: PendingPrune = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn tool_parameter_entry_serde_roundtrip() {
        let e = ToolParameterEntry {
            tool: "read_file".into(),
            signature: "read_file::{\"path\":\"x\"}".into(),
            status: Some(ToolStatus::Completed),
            turn: 2,
            message_id: "raw-msg".into(),
            result_message_id: Some("raw-result".into()),
            paths: vec!["x".into()],
            token_count: Some(128),
        };
        let back: ToolParameterEntry =
            serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn message_id_state_serde_roundtrip() {
        let mut s = MessageIdState {
            next_ref: 4,
            ..MessageIdState::default()
        };
        s.by_raw_id.insert("raw-1".into(), "m0001".into());
        s.by_ref.insert("m0001".into(), "raw-1".into());
        let back: MessageIdState =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }

    // ----- PrunedMessageEntry / PruneMessagesState / Prune -----

    #[test]
    fn pruned_message_entry_serde_roundtrip() {
        let e = PrunedMessageEntry {
            replacement_text: "[redacted]".into(),
            tokens_saved: 10,
        };
        let back: PrunedMessageEntry =
            serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn prune_messages_state_serde_roundtrip() {
        let mut s = PruneMessagesState::default();
        let block = CompressionBlock::new(
            BlockId::new(1),
            RunId::new(1),
            CompressionMode::Range,
            "t",
            "s",
            "m0001",
            "m0002",
            "raw1",
            "raw2",
        );
        s.blocks_by_id.insert(block.block_id, block.clone());
        s.active_block_ids.insert(block.block_id);
        s.next_block_id = BlockId::new(2);
        s.next_run_id = RunId::new(2);
        s.frontier_message_ref = Some("m0010".into());
        let back: PruneMessagesState =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn prune_serde_roundtrip() {
        let mut p = Prune::default();
        p.tools.insert("c1".into(), 50);
        p.messages.next_block_id = BlockId::new(3);
        let back: Prune = serde_json::from_str(&serde_json::to_string(&p).unwrap()).unwrap();
        assert_eq!(p, back);
    }

    // ----- Nudges / CompressionTimingState -----

    #[test]
    fn nudges_serde_roundtrip() {
        let mut n = Nudges {
            context_limit_counter: 2,
            iteration_counter: 1,
            ..Nudges::default()
        };
        n.turn_nudged_pairs.insert(("u1".into(), "a1".into()));
        let back: Nudges = serde_json::from_str(&serde_json::to_string(&n).unwrap()).unwrap();
        assert_eq!(n, back);
    }

    #[test]
    fn compression_timing_state_serde_roundtrip() {
        let t = CompressionTimingState {
            total_compress_ms: 250,
            samples: 4,
        };
        let back: CompressionTimingState =
            serde_json::from_str(&serde_json::to_string(&t).unwrap()).unwrap();
        assert_eq!(t, back);
    }

    // ----- SessionState -----

    #[test]
    fn session_state_default_is_serde_roundtrip() {
        let s = SessionState::default();
        let back: SessionState = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn session_state_populated_serde_roundtrip() {
        let mut s = SessionState {
            session_id: Some("session-1".into()),
            is_subagent: false,
            compress_permission: CompressPermission::Ask,
            current_turn: 3,
            last_message_was_assistant_text: true,
            last_apply_turn: Some(2),
            force_apply_requested: false,
            model_context_limit: Some(200_000),
            system_prompt_tokens: Some(2500),
            pending_prune: Some(PendingPrune {
                tool_ids: vec!["c1".into()],
                cumulative_tokens: 64,
                accumulated_at_turn: 3,
            }),
            ..SessionState::default()
        };
        s.manual_mode.enabled = true;
        s.tool_id_list.push("c1".into());
        s.tool_parameters.insert(
            "c1".into(),
            ToolParameterEntry {
                tool: "read_file".into(),
                signature: "read_file::{}".into(),
                status: Some(ToolStatus::Completed),
                turn: 1,
                message_id: "raw1".into(),
                result_message_id: None,
                paths: vec![],
                token_count: Some(64),
            },
        );
        s.stats.dedup_pruned = 1;
        s.nudges.context_limit_counter = 1;
        let json = serde_json::to_string(&s).unwrap();
        let back: SessionState = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    // ----- MessageRefKind -----

    #[test]
    fn message_ref_kind_serde_roundtrip() {
        let m = MessageRefKind::Message(42);
        let s = serde_json::to_string(&m).unwrap();
        let back: MessageRefKind = serde_json::from_str(&s).unwrap();
        assert_eq!(m, back);

        let b = MessageRefKind::Block(BlockId::new(7));
        let s = serde_json::to_string(&b).unwrap();
        let back: MessageRefKind = serde_json::from_str(&s).unwrap();
        assert_eq!(b, back);
    }
}
