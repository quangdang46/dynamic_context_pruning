# SPEC.md — Dynamic Context Pruning

> **Repo**: `dynamic_context_pruning`
> **License**: MIT
> **Document status**: v1.0 — clean-room behavior specification, locked.
> **Audience**: Implementers writing the Rust code from scratch.

This document is the **single source of truth** for the runtime behavior of the
`dynamic_context_pruning` library. It is written in prose plus pseudocode and is
intentionally implementation-language-agnostic. The Rust crate is one valid
implementation; any other implementation that satisfies every requirement,
invariant, and test fixture defined here is also conformant.

This document was authored independently from public design notes and research
summaries. It does **not** reference, copy, or derive structure from any
prior codebase in any language. Pseudocode in this file is illustrative only
and must be re-expressed by the implementer in idiomatic target-language code.

---

## Table of contents

1. Glossary
2. Canonical IR
3. Session lifecycle
4. Tool tracking
5. Pruning strategies
   - 5.1 Deduplicate
   - 5.2 Purge errored tool inputs
   - 5.3 Stale file reads
6. Compression
   - 6.1 Range mode
   - 6.2 Message mode
   - 6.3 Block bookkeeping
   - 6.4 Nesting and consumption
   - 6.5 Frontier mechanism
7. Cache stability
   - 7.1 Mode definitions
   - 7.2 Pending state semantics
   - 7.3 Apply triggers
8. Nudges
   - 8.1 Context-limit nudge
   - 8.2 Turn nudge
   - 8.3 Iteration nudge
9. Persistence
   - 9.1 Schema V1
   - 9.2 Migration rules
   - 9.3 Atomic write protocol
10. Configuration
    - 10.1 Cascade order
    - 10.2 Field semantics
    - 10.3 Validation rules
11. Edge cases and invariants
12. Test fixtures (coverage matrix)

---

## 1. Glossary

This section defines every term used normatively in this document. When a term
is written in **bold** elsewhere, the definition here applies verbatim.

### 1.1 Term definitions

| Term | Definition |
|------|------------|
| **Message** | A single record in the conversation, identified by an opaque `id`, carrying a `role`, a list of `parts`, and a timestamp. Messages are immutable from the library's perspective: the library never mutates the original input message stream; transformations always produce a new sequence. |
| **Part** | A typed fragment of a message's payload. Each part is one of: text, reasoning, tool call, tool result, or image. A message is the ordered concatenation of its parts. |
| **Role** | The producer of a message. One of `user`, `assistant`, or `system`. The library only emits `user` and `assistant` messages in transformed output; `system` content is handled through a separate prompt-injection path. |
| **Tool call** | An assistant-emitted part that requests execution of a named tool with structured input. Identified by a `call_id` that is unique within a session. |
| **Tool result** | A user-emitted part that carries the output (or error) of a previously emitted tool call, linked by `call_id`. |
| **Turn** | A logical unit of one user request followed by one or more assistant responses, possibly interleaved with tool calls and tool results, ending when the assistant emits a message that contains text and contains no pending tool call. The library tracks a monotonically increasing `current_turn` counter that advances at each turn boundary. |
| **Block** | A unit of compressed conversation. A block replaces a contiguous range (range mode) or a set of individual messages (message mode) with a single summary string. Identified by a numeric `block_id` that is monotonically increasing within a session. |
| **Run** | A grouping of blocks created by a single invocation of the compress tool. Identified by a numeric `run_id`, monotonically increasing within a session. One run may produce one or more blocks. |
| **Anchor** | The single message that visually represents a block in transformed output. The anchor is the first non-pruned message in the block's covered range. The block's summary is injected into (or alongside) the anchor message; all other messages in the range are removed from the transformed output. |
| **Compaction** | A coarse-grained event in which the host collapses or rewrites the conversation history (typically performed by the LLM provider or by a host-level summarizer). The library detects compaction by observing message-id discontinuities and resets caches accordingly. Compaction is *external* to the library; the library does not perform compaction itself. |
| **Prune** | A fine-grained, deterministic decision to drop or rewrite content within an existing message, without removing the message envelope. Pruning preserves message ids, role, and tool-call/tool-result pairing. The library performs three kinds of pruning automatically (see Section 5). |
| **Frontier** | The boundary between content that is eligible for further compression and content that has been deemed not worth re-compressing. After a compress attempt produces a summary that is larger than the raw range it would replace, the frontier advances past that range, preventing the model from being repeatedly nudged to compress the same span. |
| **Nudge** | A short instruction injected into the transformed message stream that asks the model to invoke the compress tool. There are three kinds: context-limit, turn, and iteration. Nudges are placed near specific anchor messages and may be `soft` or `strong`. |
| **Reference** | An opaque identifier the library exposes to the model so the model can name messages or blocks when calling compress. Message references have the form `m####` (zero-padded four-digit decimal); block references have the form `b#` (no padding). References are stable across the session — once allocated, a reference always denotes the same underlying message or block. |
| **Signature** | A canonical string derived from a tool name and its normalized parameters, used for deduplication. Two tool calls have the same signature iff the library should treat their inputs as logically equivalent. |
| **Protected** | Marked as exempt from a particular kind of pruning. Protection can apply to tool names, file paths (via glob patterns), tags inside text, or whole user messages. Protection is always opt-in and configured per strategy. |

### 1.2 Symbols and notation

Throughout pseudocode:

- `state` refers to the in-memory `SessionState` (Section 9.1 names every field
  explicitly; pseudocode uses dotted-path notation such as
  `state.prune.tools`).
- `config` refers to the resolved configuration object (Section 10).
- `messages` is an ordered list of canonical messages.
- `tokens(s)` denotes the configured tokenizer's token count for string `s`.
- `now()` denotes the current wall-clock instant in milliseconds since the Unix
  epoch.
- Numbered steps are mandatory and ordered; conditions on a step are part of
  that step.

---

## 2. Canonical IR

The library operates on a canonical internal representation (IR). All host
formats (provider-specific message shapes, framework message shapes) must be
converted to the IR before being passed in, and converted back from the IR
before being shipped to a provider.

### 2.1 Message shape

A message has exactly four logical fields:

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | yes | Opaque, host-assigned, unique within the session. Used as the dictionary key everywhere internal state references a message. The library does not interpret the format. |
| `role` | enum | yes | One of `user`, `assistant`, `system`. |
| `parts` | ordered list of Part | yes | At least one element after validation. |
| `time` | integer milliseconds | yes | May be `0` if the host has no timestamp. Used only for telemetry and tie-breaking, never for logic that affects pruning. |

### 2.2 Part variants

A part is exactly one of the variants in the table below. Each variant has its
own required fields. No variant carries an id of its own except where noted.

| Variant | Required fields | Notes |
|---------|----------------|-------|
| `text` | `text: string` | The user-visible text body of the message. May be empty. |
| `reasoning` | `text: string` | Internal chain-of-thought, separate from `text`. The library treats reasoning as content for token counting but never modifies it during pruning. |
| `tool_call` | `call_id: string`, `tool: string`, `input: json value` | `call_id` must be unique within the session. `input` is an arbitrary JSON value; the library normalizes it for signature purposes (Section 4.4). |
| `tool_result` | `call_id: string`, `status: enum`, `output: optional string`, `error: optional string` | `call_id` must match a previously seen `tool_call`. Status is one of `pending`, `running`, `completed`, `error`. |
| `image` | `media_type: string`, `data: string` | `data` is a base64-encoded payload. The library counts a fixed token cost per image (configurable, default 1500) and otherwise treats images as opaque. |

### 2.3 Role semantics

| Role | Must contain | May contain | Library behavior |
|------|--------------|-------------|------------------|
| `user` | At least one of: `text`, `tool_result`, `image` | Multiple `tool_result` parts | Used to anchor the start of a turn. User messages are never compressed unless explicitly enabled by `compress.protectUserMessages == false` and only via explicit compress invocations. |
| `assistant` | At least one of: `text`, `reasoning`, `tool_call` | Mixed `text` + `tool_call` in the same message | Used to detect turn boundaries (Section 3.2). The library may inject nudges into assistant messages only as guidance text; it never invents tool calls. |
| `system` | `text` | — | The library never emits or rewrites system messages directly. The `transform_system` entry point appends library-controlled prompt fragments to a host-provided system string. |

### 2.4 ID rules

Identifiers are partitioned into four namespaces. Each namespace has its own
allocation rule; no two identifiers in different namespaces ever collide because
their format prefixes differ.

| Namespace | Format | Allocation | Stability |
|-----------|--------|-----------|-----------|
| Message id (raw, host-assigned) | opaque string | host | Lifetime of the session; the library treats raw ids as keys. |
| Message reference (library-assigned) | `m` followed by zero-padded 4-digit decimal, e.g. `m0001`–`m9999` | library, in order of first appearance | Permanent for the session once allocated. |
| Block id | positive integer (`u32` semantically), exposed as `b<n>` (no padding) | library, monotonically increasing per session | Permanent. Never reused even if the block is deactivated. |
| Run id | positive integer (`u32` semantically) | library, monotonically increasing per session | Permanent. One run produces one or more blocks. |
| Tool call id | opaque string, host-assigned | host | Permanent. Used to pair calls with results. |

#### Allocation procedures

**Message reference allocation** (`allocate_next_message_ref`):

1. Read `state.message_ids.next_ref`. If it is `0`, set it to `1`.
2. If `state.message_ids.next_ref > 9999`, return an error
   `MessageRefExhausted` and stop. (See Section 11.7 for behavior at this
   limit.)
3. Format the string `m{:04}` from the current value.
4. Increment `state.message_ids.next_ref` by `1`.
5. Return the formatted string.

**Block id allocation** (`allocate_block_id`):

1. Read `state.prune.messages.next_block_id`. If it is `0`, set it to `1`.
2. Capture the current value as `id`.
3. Increment `state.prune.messages.next_block_id` by `1`.
4. Return `id`.

**Run id allocation** (`allocate_run_id`):

1. Read `state.prune.messages.next_run_id`. If it is `0`, set it to `1`.
2. Capture the current value as `id`.
3. Increment `state.prune.messages.next_run_id` by `1`.
4. Return `id`.

#### Reference exposure

When the library prepares messages for the LLM, it appends a single XML-tagged
identifier line at the end of each message's primary text part:

```
<dcp-message-id>m0042</dcp-message-id>
```

The exact tag name is fixed (`dcp-message-id`). When a message has no text
part, the library inserts a synthetic text part containing only the tag.
Block references (`b<n>`) are exposed only inside compression block summaries
(Section 6.3) and never as standalone tags on plain messages.

### 2.5 Validation rules for input messages

Before any processing, every input message must pass the validation in this
table. Failed messages are dropped silently and a counter
`stats.dropped_invalid` is incremented.

| Check | Rule |
|-------|------|
| Non-empty parts | `parts` has length ≥ 1. |
| Role consistency | If `role == user`, no part is `tool_call` or `reasoning`. If `role == assistant`, no part is `tool_result`. |
| Tool result pairing | Every `tool_result` part references a `call_id` that has appeared earlier in the message stream as a `tool_call`. Unmatched results are dropped (the message envelope is kept if it has other parts; otherwise the entire message is dropped). |
| Id uniqueness | A message's `id` does not collide with any other message already accepted in the same session. Duplicates are dropped. |
| UTF-8 well-formedness | All string fields are valid UTF-8. The library never produces invalid UTF-8 in output. |

---

## 3. Session lifecycle

A session is the unit of state that the library maintains for one ongoing
conversation. Each session has a unique `session_id` (opaque host-provided
string, typically derived from the conversation's persistent identifier).

### 3.1 Session start

Triggered by: the host constructs a `ContextPruner` and the first call to
`transform_messages` is made with a non-empty input.

Inputs:

- The configuration object (Section 10).
- The message list passed to the first `transform_messages` call.
- Optional persisted state loaded from the storage backend.

Outputs (mutations to state):

- `state.session_id` is set to the host-assigned id (or, if the host did not
  set one, derived deterministically from the last message's id).
- `state.current_turn` is set to the count of completed turns observable in the
  input (see Section 3.2 for the counting rule).
- All persisted compression blocks (if any) are loaded into
  `state.prune.messages.blocks_by_id`, with `next_block_id` and `next_run_id`
  advanced past the maximum loaded value.
- Message-reference allocations are replayed by walking the input messages in
  order and assigning `m####` references to non-ignored messages.

Pseudocode:

1. If the storage backend has a persisted state for `session_id`, deserialize
   it (Section 9.1) and apply migration if the schema version is older
   (Section 9.2).
2. Initialize `state.message_ids.next_ref` to `1` if not present in persisted
   state; otherwise to `max(existing m####) + 1`.
3. Initialize `state.prune.messages.next_block_id` to `1` if no persisted
   blocks; otherwise to `max(block_id) + 1`.
4. Initialize `state.prune.messages.next_run_id` to `1` if no persisted runs;
   otherwise to `max(run_id) + 1`.
5. Set `state.last_message_was_assistant_text = false`.
6. Set `state.current_turn = 0`.
7. Validate every input message per Section 2.5; drop invalid ones.
8. Walk validated messages and assign references (Section 2.4).
9. Walk validated messages and rebuild the tool-tracking dictionaries
   (Section 4).
10. The session is now ready; `transform_messages` may proceed.

Pre-conditions:

- `config` has passed validation (Section 10.3).
- The storage backend, if configured, is reachable for read.

Post-conditions:

- `state.session_id` is set.
- All references in input messages have been assigned.
- All previously-persisted blocks are loaded into memory.

Edge cases:

- Empty input list: skip steps 7–9; the session is initialized but transforms
  to an empty list. Subsequent calls re-enter session-start logic until
  messages exist.
- Persisted state with a newer schema version than the running library:
  return `Error::PersistenceVersionTooNew` and refuse to start.
- Persisted state references block ids whose anchor messages no longer exist
  in the input: deactivate those blocks (set `active = false`,
  `deactivated_at = now()`, `deactivated_by_block_id = None`) and continue.
- Persisted state has duplicate block ids: load the first one encountered,
  drop subsequent duplicates, and increment `stats.persisted_corruption`.
- Storage read fails: log the error, proceed with a fresh in-memory state,
  set `stats.storage_load_failed = true`. The library does not abort.

### 3.2 Turn boundaries

A turn begins at every `user`-role message and ends at the next `assistant`
message that satisfies all of:

- Contains at least one `text` part.
- Contains zero `tool_call` parts that are not yet matched by a `tool_result`.

When detection emits a turn-end, `state.current_turn` is incremented by `1`
and `state.last_message_was_assistant_text` is set to `true`. Any other
configuration of trailing message resets
`state.last_message_was_assistant_text` to `false` without changing
`current_turn`.

Pseudocode (`detect_turn_boundary`):

1. Let `last` be the final validated message in the input.
2. If `last.role != assistant`, set
   `state.last_message_was_assistant_text = false` and return.
3. Let `has_text = any part is text` in `last.parts`.
4. Let `has_open_call = any part is tool_call whose call_id has no matching
   tool_result in any subsequent or same message`. (Tool-call/result pairing
   is computed in Section 4.2.)
5. If `has_text` and not `has_open_call`, set
   `state.last_message_was_assistant_text = true`. Increment
   `state.current_turn` if this turn-end was not already counted (a per-id
   memo prevents double counting on repeated calls).
6. Otherwise, set `state.last_message_was_assistant_text = false`.

Edge cases:

- An assistant message with `text` *and* an unanswered `tool_call`: this is
  mid-turn, not turn-end. `last_message_was_assistant_text` is set to false.
- An assistant message with `tool_call` only and no text: mid-turn.
- A user message at the end of the list: not a turn-end.
- A tool_result at the end (in a user-role message): mid-turn; the assistant
  has not yet responded.
- Two consecutive assistant text messages without an intervening user message:
  treat the last one as turn-end; `current_turn` advances exactly once for
  the pair (memo by message id).

### 3.3 Compaction detection

The library does not perform compaction. It detects when the host or provider
has performed compaction by observing message-id discontinuities.

Detection rule: at the start of every `transform_messages` invocation, compare
the set of message ids seen in this call against `state.message_ids.by_raw_id`
keys. If the new input does **not** contain at least one of the previously
referenced ids that ought to still be present (heuristic: at least one of the
last three referenced ids), classify this as a compaction event.

Pseudocode (`detect_compaction`):

1. Let `referenced_ids` be the set of raw message ids previously assigned
   references that the library still considers "live" (i.e. not consumed by a
   compression block).
2. Let `seen_ids` be the set of raw message ids in the current input.
3. If `referenced_ids` is empty, return `false`.
4. Compute the most recent up-to-three live referenced ids
   (`recent_referenced`).
5. If none of `recent_referenced` are present in `seen_ids`, return `true`.
6. Otherwise return `false`.

When a compaction event is detected:

1. Clear `state.tool_parameters` and `state.tool_id_list`.
2. Clear `state.message_ids.by_raw_id` and `state.message_ids.by_ref`. Reset
   `next_ref` to `1`.
3. Mark all currently-active compression blocks as `active = false`,
   `deactivated_at = now()`, `deactivated_by_block_id = None`. Keep them in
   `blocks_by_id` for audit.
4. Reset `state.prune.tools`.
5. Increment `state.last_compaction` to `now()`.
6. Increment `stats.compactions_observed`.
7. Re-run session-start steps 7–9 against the new input.

Edge cases:

- The first call ever: `referenced_ids` is empty; not a compaction.
- A partial truncation that drops only the very oldest messages but keeps
  recent ids: not flagged as compaction (the heuristic looks at the *recent*
  three, which are still present).
- Compaction concurrent with a pending compress run: the pending plan is
  abandoned. Allocated block ids that did not commit are released only by
  *not* counting them in `next_block_id` advancement (allocation is
  best-effort idempotent: the implementation may either roll back or simply
  let the id remain unused; both are conformant).

### 3.4 Session end

A session does not have an explicit end signal in the API; the library treats
each `ContextPruner` instance as owning the session for its lifetime. The host
ends a session by:

- Calling `save()` to flush state to the storage backend, then dropping the
  pruner instance, or
- Calling `reset()` to clear in-memory state without writing to storage.

Pseudocode (`save`):

1. Construct a `PersistedStateV1` from current `state` per Section 9.1.
2. Serialize to JSON.
3. Atomic-write to the storage backend (Section 9.3).
4. On success, set `state.last_persisted_at = now()` and return Ok.
5. On failure, increment `stats.storage_save_failed` and return the error.

Pseudocode (`reset`):

1. Replace `state` with a default-constructed `SessionState`.
2. Storage is not modified. The on-disk state remains until overwritten.

Edge cases:

- `save()` called with no mutations since last save: still writes a fresh
  file with an updated `last_updated` timestamp. Atomic write ensures no
  corruption.
- `reset()` called mid-transform: undefined; the host is responsible for not
  doing this.
- Process crash before `save()`: latest persisted state is from the most
  recent successful save. The library's idempotent rebuild (Section 11.4)
  recovers the rest from the message stream.

---

## 4. Tool tracking

Tool tracking maintains the per-session bookkeeping needed by the deduplicate,
purge-errors, and stale-file-reads strategies. Every tool call observed in the
input is added to `state.tool_parameters` and `state.tool_id_list`.

### 4.1 Tool parameter entry

Each entry has the following fields:

| Field | Type | Description |
|-------|------|-------------|
| `call_id` | string | Host-assigned. Key into the dictionary. |
| `tool` | string | Tool name, e.g. `read`, `write`, `bash`. |
| `parameters` | json value | The original `input` JSON, unchanged. |
| `normalized` | json value | `parameters` after normalization (Section 4.4). |
| `signature` | string | Computed signature (Section 4.5). |
| `status` | enum | One of `pending`, `running`, `completed`, `error`. Updated as `tool_result` parts are observed. |
| `turn` | u32 | The value of `state.current_turn` at the time the tool call was first observed. |
| `message_id` | string | Raw id of the assistant message that emitted the call. |
| `result_message_id` | optional string | Raw id of the user message that emitted the matching result. `None` until the result arrives. |
| `token_count` | optional u64 | Token count of the verbatim tool call + result text (may be filled lazily; missing means "unknown"). |
| `paths` | list of string | File paths extracted from `parameters` per Section 4.6. May be empty. |

### 4.2 Call/result pairing

Pairing rules:

1. Whenever the input contains an assistant `tool_call` part with `call_id =
   c`, allocate or update an entry keyed by `c`.
2. Whenever the input contains a user `tool_result` part with `call_id = c`:
   - If no entry exists for `c`, drop the result and increment
     `stats.orphan_tool_results`.
   - Otherwise, set `entry.status` from the result's `status` field and set
     `entry.result_message_id` to the containing message's raw id.

A tool call is **paired** if and only if `entry.result_message_id.is_some()`.

The library never separates a paired call from its result: any pruning,
compression, or filtering operation that drops a tool_call must also drop the
matching tool_result (and vice versa); see Section 11.1.

### 4.3 Status transitions

Allowed transitions (rows = current, columns = next):

|              | pending | running | completed | error |
|--------------|---------|---------|-----------|-------|
| (none)       | ✓       | ✓       | ✓         | ✓     |
| pending      | ✓       | ✓       | ✓         | ✓     |
| running      |         | ✓       | ✓         | ✓     |
| completed    |         |         | ✓         |       |
| error        |         |         |           | ✓     |

Invalid transitions (e.g. `completed → error`) are silently ignored; the
existing status is preserved and `stats.invalid_status_transitions` is
incremented.

### 4.4 Parameter normalization

Normalization removes incidental differences in JSON inputs so that two
logically equivalent calls produce the same signature.

Algorithm (`normalize`):

1. If the value is `null` or a primitive (boolean, number, string), return it
   unchanged. Numbers are not coerced (e.g. `1` and `1.0` remain distinct;
   most JSON parsers preserve this distinction, and the library does not
   collapse it).
2. If the value is an array, recursively normalize each element. The order is
   preserved.
3. If the value is an object:
   a. Drop every key whose value is `undefined` (in JSON: explicitly `null`
      *only if* the key name is in the configured `drop_null_keys` list;
      default empty — i.e. `null` values are kept).
   b. Recursively normalize each remaining value.
   c. Sort keys lexicographically (UTF-8 byte order).
4. Return the resulting value.

Edge cases:

- Whitespace inside string values is preserved verbatim. Two calls that differ
  only in trailing whitespace inside a parameter are *not* deduplicated.
- Cyclic JSON values cannot exist (JSON has no cycles), so recursion always
  terminates.
- Very deep nesting (>1000 levels) returns the value unchanged and increments
  `stats.normalize_depth_clamped` rather than recursing further.
- Numeric precision differences across producers (e.g. `0.1 + 0.2`) are out of
  scope; signature equality requires byte-equal serialization after sorting.
- Keys that differ only by Unicode normalization form (NFC vs. NFD) are
  treated as distinct; the library does not Unicode-normalize keys.

### 4.5 Signature computation

The signature is a single string of the form:

```
<tool_name>::<canonical_json>
```

where `canonical_json` is the JSON serialization of the normalized parameters
with these rules:

- No whitespace between tokens.
- Object keys are quoted strings in lexicographic order.
- Arrays preserve element order.
- Strings are escaped with the minimal set of escapes required by RFC 8259.
- Floating-point numbers use the shortest round-trip decimal representation
  (no engineering notation unless the absolute value is ≥ 1e21 or < 1e-6).

Two calls share a signature iff their tool names are byte-equal and their
canonical-json forms are byte-equal.

### 4.6 File path extraction

For tools that operate on files, the library extracts a list of file paths
from the parameters. The extraction is purely structural and does not require
the tool to actually exist on disk.

Rules:

| Tool name (case-sensitive) | Path keys (in priority order) |
|----------------------------|-------------------------------|
| `read` | `path`, `file_path`, `filename` |
| `write` | `path`, `file_path`, `filename` |
| `edit` | `path`, `file_path`, `filename` |
| `multiedit` | `path`, `file_path`, `filename` (single) plus `edits[].path` (multi) |
| any other | none |

Only the `tracked_tools` list (configurable, Section 10.2) is consulted; tools
not in the list have an empty `paths` list regardless of input shape.

Extraction algorithm (`extract_file_paths`):

1. If `tool` is not in the configured `tracked_tools` list, return `[]`.
2. For each candidate key in the priority order for `tool`:
   - If `parameters[key]` is a non-empty string, push it to the result.
3. If `tool == multiedit` and `parameters.edits` is an array, for each element
   that is an object with a string `path` field, push the path to the result.
4. Deduplicate the result (preserving first occurrence).
5. Normalize paths: strip a single leading `./` if present; collapse repeated
   internal `/` to a single `/`. Do **not** resolve `..` or symlinks; the
   library treats paths textually.
6. Return the list.

Edge cases:

- A tool listed in `tracked_tools` whose parameters contain no path key:
  empty list. The tool entry exists but has no associated paths.
- Absolute vs. relative paths: kept as-is. Two calls with `/abs/foo.rs` and
  `foo.rs` are not considered to be the same file.
- Paths containing `\\` (backslashes on Windows-style hosts): kept as-is. The
  library does not translate path separators.
- Path with embedded null byte: stripped before push (and counter
  `stats.path_null_byte_stripped` incremented).
- Path longer than 4096 bytes: truncated to 4096 bytes at a UTF-8 boundary
  (Section 11.3) and pushed.

### 4.7 Building the tool id list

`state.tool_id_list` is the ordered list of `call_id` values in the order they
first appear in the input. It is rebuilt at the start of every
`transform_messages` call (full rebuild, not incremental).

Pseudocode (`build_tool_id_list`):

1. Clear `state.tool_id_list`.
2. Walk validated messages in order.
3. For each `tool_call` part with `call_id = c`:
   - If `c` is not already in the list, append it.

Post-condition: every key in `state.tool_parameters` appears at most once in
`state.tool_id_list`, in deterministic order.


---

## 5. Pruning strategies

The library applies three deterministic pruning strategies on the hot path,
in fixed order: **deduplicate**, **purge errors**, **stale file reads**.
Strategies write their decisions into `state.prune.tools` (a map from
`call_id` to estimated tokens saved). The decisions are **not** applied to
the outgoing message stream until the apply phase (Section 6 of the pipeline
in 5.4); apply phase respects cache stability (Section 7).

Every strategy returns a `PruneOutcome` describing what it did:

| Field | Type | Meaning |
|-------|------|---------|
| `name` | string | Strategy name. |
| `pruned_count` | usize | Number of tool entries newly added to `state.prune.tools` by this run. |
| `skipped_reason` | optional string | Set when the strategy did not run (e.g. `"manual_mode"`, `"disabled"`). |
| `tokens_saved` | u64 | Sum of `token_count` over the newly pruned entries. |

### 5.1 Strategy: deduplicate

**Purpose**: when the model issues several tool calls that are logically
identical (same tool, same normalized input), keep only the most recent one.
Older duplicates have nothing to add and waste tokens.

#### Inputs

| Input | Source |
|-------|--------|
| `state.tool_parameters` | Mutating dictionary, read |
| `state.tool_id_list` | Read |
| `state.prune.tools` | Read & write |
| `state.manual_mode` | Read |
| `state.current_turn` | Read |
| `config.cache_stability_mode` | Indirect (gating done by pipeline) |
| `config.manual_mode.automatic_strategies` | Read |
| `config.strategies.deduplication.enabled` | Read |
| `config.strategies.deduplication.protected_tools` | Read |
| `config.protected_file_patterns` | Read |
| `messages` | Read (only for path resolution context, optional) |

#### Outputs

- Mutations to `state.prune.tools`: new entries inserted with key `call_id`
  and value equal to that entry's `token_count` (or `0` if unknown).
- Increments `stats.dedup_pruned` by the number of entries added.
- Returns `PruneOutcome { name: "deduplicate", … }`.

#### Pseudocode

1. If `config.strategies.deduplication.enabled == false`, return
   `PruneOutcome::skipped("disabled")`.
2. If `state.manual_mode.enabled` and not
   `config.manual_mode.automatic_strategies`, return
   `PruneOutcome::skipped("manual_mode")`.
3. Initialize an empty multimap `groups: signature -> [call_id, …]`.
4. For each `id` in `state.tool_id_list`, in order:
   a. If `id` is already a key in `state.prune.tools`, skip (already pruned).
   b. Read `entry = state.tool_parameters[id]`.
   c. If `entry.tool` appears in
      `config.strategies.deduplication.protected_tools`, skip.
   d. If any path in `entry.paths` matches any glob in
      `config.protected_file_patterns`, skip.
   e. Only group entries whose `status == completed`. Skip entries whose
      status is `error`, `pending`, or `running` — these are handled by other
      strategies, and pruning a non-completed call would lose information the
      model still needs.
   f. Append `id` to `groups[entry.signature]`.
5. For each group with two or more ids, mark every id in the group **except
   the last** for pruning.
6. For each id `p` marked for pruning:
   a. Insert into `state.prune.tools` the pair `(p, entry.token_count or 0)`.
   b. Add the value to a running total `tokens_saved`.
7. Increment `stats.dedup_pruned` by the count of marks.
8. Return `PruneOutcome { name: "deduplicate", pruned_count, tokens_saved }`.

#### Pre-conditions

- `state.tool_parameters` has up-to-date `signature` and `paths` for every
  entry.
- `state.tool_id_list` is in deterministic insertion order.

#### Post-conditions

- For every signature group, at most one tool call remains unpruned (the most
  recent in `tool_id_list` order).
- No tool whose name is in `protected_tools` is added to `state.prune.tools`.
- No tool whose `paths` intersect protected globs is added.

#### Invariants preserved

- Tool call/result pairing (Section 11.1): a pruned call's matching result is
  also pruned by the apply phase, never one without the other.
- The latest call/result pair in each signature group is preserved verbatim.
- Errored tool calls are never pruned by deduplicate (purge_errors handles
  them).

#### Edge cases

- Single occurrence of a signature: nothing pruned.
- All entries in a signature group are protected: nothing pruned.
- Tool name with empty parameters object: signature is `tool::{}`; multiple
  parameter-less calls of the same tool are dedup-eligible.
- Tool result returned `error`: skipped (not in any signature group).
- The most recent call in a group has `status == pending`: it is not in any
  group (only completed entries are grouped); dedup ignores the entire group
  this turn.

### 5.2 Strategy: purge errored tool inputs

**Purpose**: tool calls that errored often carry large inputs (pasted code,
multi-megabyte logs). The error message is small and useful; the input is
large and obsolete after a few turns. This strategy deletes the input while
keeping the error result.

#### Inputs

| Input | Source |
|-------|--------|
| `state.tool_parameters` | Read |
| `state.tool_id_list` | Read |
| `state.prune.tools` | Read & write |
| `state.current_turn` | Read |
| `state.manual_mode` | Read |
| `config.strategies.purge_errors.enabled` | Read |
| `config.strategies.purge_errors.turns` | Read (turn-age threshold) |
| `config.strategies.purge_errors.protected_tools` | Read |

#### Outputs

- Inserts entries into `state.prune.tools`. The apply phase replaces the tool
  *input* with the placeholder string `[input removed due to failed tool
  call]` and leaves the tool *result* (the error message) in place.
- Increments `stats.purge_errors_pruned`.

#### Pseudocode

1. If `config.strategies.purge_errors.enabled == false`, return
   `PruneOutcome::skipped("disabled")`.
2. If `state.manual_mode.enabled` and not
   `config.manual_mode.automatic_strategies`, return skipped.
3. Let `threshold = max(1, config.strategies.purge_errors.turns)`.
4. For each `id` in `state.tool_id_list`:
   a. If `id` is already in `state.prune.tools`, skip.
   b. Read `entry = state.tool_parameters[id]`.
   c. If `entry.status != error`, skip.
   d. If `entry.tool` is in
      `config.strategies.purge_errors.protected_tools`, skip.
   e. Compute `age = state.current_turn - entry.turn`.
   f. If `age < threshold`, skip (not old enough).
   g. Mark for pruning.
5. For each marked id, insert into `state.prune.tools` with the entry's
   token_count.
6. Return `PruneOutcome { name: "purge_errors", … }`.

#### Pre-conditions

- `state.current_turn` is up to date (Section 3.2).
- `entry.turn` was set at the time the call was first observed.

#### Post-conditions

- Every errored tool call older than `threshold` turns is in
  `state.prune.tools` (unless protected).
- Errors younger than `threshold` are untouched.

#### Edge cases

- `threshold = 0` in config: clamped to `1` (errors are still kept for the
  current turn so the model can react).
- An error transitions to a non-error status (rare; happens if a host retries
  and updates the entry): the entry's status update prevents future
  purge-errors marking, but if it was already pruned the prune persists.
- Multiple errored calls of the same tool name: each is evaluated
  independently against `threshold`.
- An error in a protected tool: never pruned, regardless of age.
- A retried call (new `call_id`, same tool, same params) that succeeds: the
  successful call is unaffected; only the original errored entry is purged.

### 5.3 Strategy: stale file reads

**Purpose**: when the model reads, writes, or edits the same file repeatedly,
older copies of the file's contents become stale. Keep the most recent
version of each path; prune the rest.

This strategy is distinct from deduplicate because two calls on the same path
may have different parameters (for example, different `offset`/`limit`
values). Deduplicate would not match them; stale-file-reads matches by path
alone.

#### Inputs

| Input | Source |
|-------|--------|
| `state.tool_parameters` | Read |
| `state.tool_id_list` | Read |
| `state.prune.tools` | Read & write |
| `config.strategies.staleFileReads.enabled` | Read |
| `config.strategies.staleFileReads.tracked_tools` | Read (default: `["read", "write", "edit", "multiedit"]`) |
| `config.strategies.staleFileReads.protected_tools` | Read |
| `config.protected_file_patterns` | Read |

#### Outputs

- Inserts into `state.prune.tools`. The apply phase replaces the tool result
  body with a short placeholder describing why it was removed.
- Increments `stats.stale_file_reads_pruned`.

#### Pseudocode

1. If `config.strategies.staleFileReads.enabled == false`, return skipped.
2. Initialize an empty multimap `by_path: path -> [call_id, …]`.
3. For each `id` in `state.tool_id_list`:
   a. If `id` is in `state.prune.tools`, skip.
   b. Read `entry = state.tool_parameters[id]`.
   c. If `entry.tool` is not in `tracked_tools`, skip.
   d. If `entry.tool` is in `protected_tools`, skip.
   e. If `entry.status != completed`, skip (errors are handled by
      purge_errors; pending/running are not yet stale).
   f. For each path in `entry.paths`:
      - If the path matches any glob in `protected_file_patterns`, skip the
        path.
      - Append `id` to `by_path[path]`.
4. For each path with two or more ids, mark every id except the last (in
   `tool_id_list` order) for pruning.
5. Insert marked ids into `state.prune.tools`.
6. Return `PruneOutcome { name: "stale_file_reads", … }`.

#### Pre-conditions

- `entry.paths` are correctly populated by Section 4.6.
- `tool_id_list` is in deterministic order.

#### Post-conditions

- Per file path, at most one call/result pair carries the file's content; all
  earlier pairs for the same path are pruned (unless protected).
- A `multiedit` call associated with multiple paths counts as the latest for
  every one of those paths.

#### Edge cases

- Two reads of the same path with different offsets — both pruned except the
  most recent. The most recent might not contain the same content the older
  ones did; the model is implicitly told to re-read if it needs other
  ranges.
- A `write` followed by a `read` of the same path: the write is the older
  call (unless interleaving differs); the read is more recent and is kept;
  the write is pruned.
- A path appears in `protected_file_patterns`: every call referencing that
  path is exempt from this strategy. The path does not appear in `by_path`
  at all.
- A path-less call to a tracked tool: not added to `by_path`; this strategy
  is a no-op for it.
- A `multiedit` whose `edits` array references paths the parent call does
  not list: each edit path is added; the `multiedit` is kept as latest for
  all of them.

### 5.4 Pipeline: order of operations within `transform_messages`

The following pseudocode describes the full hot-path flow that calls the
strategies above. It is the canonical order; implementations must preserve
the relative order of phases. Where a phase is a no-op for a particular
configuration, it is still walked through to produce stable telemetry.

```
function transform_messages(input_messages):
    # Phase 0 — Validation and preflight
    valid = filter_valid(input_messages)             # Section 2.5
    detect_compaction(state, valid)                  # Section 3.3

    # Phase 1 — Synchronization
    detect_turn_boundary(state, valid)               # Section 3.2
    cache_system_prompt_tokens(state, valid)         # Section 8.1 helper
    assign_message_refs(state, valid)                # Section 2.4
    sync_compression_blocks(state, valid)            # Section 6.4
    sync_tool_cache(state, valid)                    # update tool_parameters
    build_tool_id_list(state, valid)                 # Section 4.7

    # Phase 2 — Subagent gate
    if state.is_subagent and not config.experimental.allow_subagents:
        return valid  # subagents bypass pruning, see 11.6

    # Phase 3 — Strategies (cache-stability gated)
    if should_apply_now(state, config, valid):       # Section 7.3
        run_strategy(deduplicate, state, valid)      # Section 5.1
        run_strategy(purge_errors, state, valid)     # Section 5.2
        run_strategy(stale_file_reads, state, valid) # Section 5.3
        commit_pending_to_outgoing(state)
    else:
        accumulate_pending(state)                    # Section 7.2

    # Phase 4 — Apply pruning to outgoing messages
    pruned = apply_prune_to_messages(valid, state, config)

    # Phase 5 — Compression block expansion
    pruned = filter_compressed_ranges(pruned, state, config)  # Section 6.4

    # Phase 6 — Subagent result inlining (parent only)
    pruned = inject_subagent_results(pruned, state)

    # Phase 7 — Reference + nudge injection
    priorities = build_priority_map(state, config, pruned)    # Section 8
    inject_nudges(pruned, priorities, state, config)
    inject_message_id_tags(pruned, state)

    # Phase 8 — Pending manual triggers (slash commands etc.)
    apply_pending_manual_trigger(state, pruned)

    # Phase 9 — Tail
    strip_internal_metadata(pruned)
    persistence.save_if_dirty(state)

    return pruned
```

The function `apply_prune_to_messages` walks the input messages and, for
every part referencing a `call_id` in `state.prune.tools`:

- For an assistant `tool_call` part: replaces `input` with the placeholder
  JSON value `{"removed": "[input removed due to failed tool call]"}` if
  the call is in `state.prune.tools` because of purge_errors; for dedup or
  stale-file the part is removed entirely.
- For a user `tool_result` part: removes the part entirely for dedup and
  stale-file (its sibling tool_call has also been removed, so the message
  may end up with zero parts and be dropped); for purge_errors, keeps the
  part with status `error` and original error string but replaces `output`
  with `None`.

If removing parts leaves a message with zero parts and the message has no
`text` part, the message is dropped from the outgoing list.


---

## 6. Compression

Compression is initiated by the LLM through a special tool the library
registers with the host's tool layer. The library exposes a tool named
`compress`. When the model calls it, the host forwards the call to
`handle_compress`, which executes the compression and returns a result the
host hands back to the model.

There are two compression modes: **range mode** (default) and **message
mode**. The active mode is controlled by `config.compress.mode`. The library
exposes only one mode at a time through the tool schema.

### 6.1 Range mode

Range mode lets the model summarize a contiguous span of conversation in one
operation. One compress invocation may submit multiple non-overlapping
ranges; each becomes a single block.

#### Tool schema

The schema (JSON Schema fragment used for tool registration) is:

```jsonc
{
  "name": "compress",
  "description": "Replace contiguous ranges of conversation history with topic summaries. Use when ranges of older messages can be condensed without losing information needed for ongoing tasks.",
  "parameters": {
    "type": "object",
    "properties": {
      "topic": {
        "type": "string",
        "description": "Short label naming the overall topic of this compression batch."
      },
      "content": {
        "type": "array",
        "minItems": 1,
        "items": {
          "type": "object",
          "properties": {
            "startId": {
              "type": "string",
              "description": "Reference of the first message or block in the range (m#### or b#)."
            },
            "endId": {
              "type": "string",
              "description": "Reference of the last message or block in the range; must be at or after startId."
            },
            "summary": {
              "type": "string",
              "description": "Self-contained summary of the range."
            }
          },
          "required": ["startId", "endId", "summary"]
        }
      }
    },
    "required": ["topic", "content"]
  }
}
```

#### Validation rules

Validation happens before any state is mutated. If any rule fails, the entire
call is rejected with a structured error and **no** state changes occur.

| Rule | Behavior on failure |
|------|---------------------|
| `topic` is non-empty after trimming | `Error::InvalidCompressArgs("empty topic")` |
| `content` is a non-empty array | `Error::InvalidCompressArgs("empty content")` |
| Each entry has `startId`, `endId`, `summary` strings | `Error::InvalidCompressArgs("malformed entry")` |
| `startId` and `endId` resolve to a known reference | `Error::InvalidCompressArgs("unknown ref: <id>")` |
| `startId <= endId` in conversation order | `Error::InvalidCompressArgs("inverted range")` |
| Ranges within a single call do not overlap | `Error::RangeOverlap(<details>)` |
| `summary` length is between 1 and `max_summary_chars` (default 32 KiB) | `Error::InvalidCompressArgs("summary too short" / "too long")` |
| `summary` placeholders, if any, all resolve | `Error::PlaceholderMismatch(<details>)` |

A reference is considered known if:

- For `m####`: it exists in `state.message_ids.by_ref`.
- For `b#`: it exists in `state.prune.messages.blocks_by_id` and is currently
  active.

#### Range resolution

For each entry:

1. Resolve `start_ref` and `end_ref`:
   - If the reference is `m####`, the corresponding raw message id is
     `state.message_ids.by_ref[ref]`.
   - If the reference is `b#`, the block's anchor message id is used as the
     boundary.
2. Compute the **selection**: the ordered list of raw message ids from the
   resolved start (inclusive) to the resolved end (inclusive), drawn from
   the *current input message stream*, excluding messages that are already
   inside an active block (those are represented only via their block).
3. Compute `required_block_ids`: every active block whose anchor message id
   falls within the selection. These are the blocks that the new compression
   will *consume*.
4. Compute the **anchor message id**: the first message id in the selection
   that is *not* the anchor of any block being consumed. If every selected
   message is consumed by a block, the anchor is the first selected message.
5. Determine `direct_message_ids`: the message ids in the selection that are
   not part of any consumed block.
6. Determine `direct_tool_ids`: the `call_id` values of any tool call/result
   parts within `direct_message_ids`.

#### Summary placeholder syntax

A summary may reference previously-compressed blocks within its text using
placeholders of the form:

```
{{block:b3}}
```

When the summary is rendered (Section 6.4 — `filter_compressed_ranges`), each
placeholder is expanded to the referenced block's full summary, surrounded by
a fenced block:

```
<dcp-block id="b3">
... summary text of b3 ...
</dcp-block>
```

Validation rules for placeholders:

- Every placeholder's referenced block id must be in
  `required_block_ids` for the entry. (The model may not reference arbitrary
  blocks outside the range; only the ones being consumed.)
- A required block id that is *not* mentioned by any placeholder is appended
  automatically by the library at the end of the new summary in deterministic
  order (block id ascending). This ensures information is never silently
  lost.
- Duplicate placeholders for the same block id are collapsed to one
  expansion.
- Placeholders for non-existent block ids fail validation.

#### Outputs (range mode)

- One new block per entry, allocated under a single `run_id`.
- Mutations to:
  - `state.prune.messages.blocks_by_id` (insert each new block)
  - `state.prune.messages.active_block_ids` (insert each new block id; remove
    consumed block ids)
  - `state.prune.messages.active_by_anchor_message_id` (rewire anchors)
  - `state.prune.messages.next_block_id`, `next_run_id`
  - For each consumed block: `block.active = false`,
    `block.deactivated_at = now()`,
    `block.deactivated_by_block_id = new_block_id`,
    `block.parent_block_ids` includes new_block_id.
- Returns `CompressResult { compressed_messages: <count>, blocks:
  [NotificationEntry, …] }` with one `NotificationEntry` per new block:
  `{ block_id, run_id, summary, summary_tokens }`.

#### Pseudocode

```
function handle_compress_range(args, raw_messages):
    validate_topic_and_content(args)
    call_id = current_tool_call_id()  # the compress call's own id

    raw, search_ctx = prepare_session(raw_messages, args.topic)
    plans = []
    for entry in args.content:
        plan = resolve_range(entry, search_ctx, state)
        plans.append(plan)

    validate_non_overlapping(plans)
    for plan in plans:
        placeholders = parse_placeholders(plan.entry.summary)
        validate_placeholders(placeholders, plan.required_block_ids)
        plan.expanded_summary = inject_placeholder_expansions(plan)
        plan.expanded_summary = append_protected_user_messages(plan)
        plan.expanded_summary = append_protected_prompt_text(plan)
        plan.expanded_summary = append_protected_tool_outputs(plan)
        plan.expanded_summary = append_missing_block_summaries(plan)

    run_id = state.allocate_run_id()
    new_blocks = []
    for plan in plans:
        block_id = state.allocate_block_id()
        wrapped = wrap_compressed_summary(block_id, plan.expanded_summary)
        summary_tokens = tokenizer.count(wrapped)
        block = build_compression_block(plan, block_id, run_id, wrapped,
                                        summary_tokens)
        commit_block(state, block, plan)  # see Section 6.4
        new_blocks.append({block_id, run_id, summary: plan.expanded_summary,
                           summary_tokens})

    finalize_session(raw, new_blocks, args.topic)
    return CompressResult{
        compressed_messages: sum(plan.direct_message_ids.len() for plan in plans),
        blocks: new_blocks
    }
```

#### Pre-conditions

- `state.message_ids` and `state.prune.messages.blocks_by_id` are
  consistent with `raw_messages`.
- The compress tool call has been registered as `state.compress_permission ==
  allow` or the host has resolved a `permission == ask` prompt.

#### Post-conditions

- For each new block: `state.prune.messages.blocks_by_id` contains the new
  entry with `active = true` and the wrapped summary stored.
- For each consumed block: deactivated as described above.
- `next_block_id > max(block_id)` and `next_run_id > run_id`.
- The `direct_message_ids` of the new block are not anchors of any active
  block.

#### Edge cases (range mode)

- Range covering an entire active block but extending beyond it: the active
  block is consumed; the new block has `included_block_ids` containing the
  consumed block id and additional `direct_message_ids` for the surrounding
  messages.
- Range exactly matching an active block: the inner block is consumed and
  effectively replaced. The new block adds no new direct messages but gives
  the model an opportunity to retitle or improve the summary.
- Range overlapping two active blocks: legal; both are consumed and the new
  block's `included_block_ids` lists both.
- Range overlapping a block but not fully containing it: rejected with
  `Error::RangeOverlap`. The model must include or exclude a block as a
  whole; partial coverage is not supported.
- Range whose endId is older than its startId: rejected as inverted range.
- Range that starts at the very first message and ends at the latest: legal,
  produces a single huge block; see frontier (Section 6.5).

### 6.2 Message mode

Message mode lets the model pick individual non-contiguous messages to
compress. Each entry produces its own block (single-message block).

#### Tool schema

```jsonc
{
  "name": "compress",
  "description": "Replace individual older messages with topic summaries.",
  "parameters": {
    "type": "object",
    "properties": {
      "topic": {"type": "string"},
      "content": {
        "type": "array",
        "minItems": 1,
        "items": {
          "type": "object",
          "properties": {
            "messageId": {"type": "string", "description": "m####"},
            "topic": {"type": "string"},
            "summary": {"type": "string"}
          },
          "required": ["messageId", "topic", "summary"]
        }
      }
    },
    "required": ["topic", "content"]
  }
}
```

#### Validation rules

| Rule | Notes |
|------|-------|
| `topic` non-empty | Same as range mode. |
| `content` non-empty array | Same. |
| `messageId` resolves in `state.message_ids.by_ref` | Required. |
| `messageId` does **not** correspond to a message currently inside an active block | The library would otherwise have two blocks claiming the same anchor. |
| `summary` length within bounds | Same default (32 KiB). |
| Each `messageId` appears at most once across `content` | Reject duplicates. |
| Per-entry `topic` is non-empty | Required for message mode (used for in-block heading). |

Message mode does **not** allow placeholders (a single message can't already
be compressed; if it were, it would be inside a block and rejected). The
library still appends protected content (Section 6.3.2).

#### Resolution

For each entry: the selection is the single referenced message; the anchor
is the same message; `direct_message_ids = [anchor]`; `direct_tool_ids` is
the `call_id` of any tool_call/tool_result parts in that message;
`included_block_ids = []`; `consumed_block_ids = []`.

#### Outputs

- One block per entry, all under a single `run_id`. Each block has
  `mode = Message`, `start_id = end_id = messageId`.
- No consumption; `parent_block_ids` is empty for every new block.
- Returns `CompressResult` analogous to range mode.

#### Pseudocode

Identical structure to `handle_compress_range`, with these differences:

- `resolve_range` is replaced by a single-message resolver.
- `validate_non_overlapping` reduces to "no duplicate `messageId`s".
- `inject_placeholder_expansions` is a no-op.
- `append_missing_block_summaries` is a no-op.

#### Edge cases (message mode)

- Message id is the most recent assistant message: the library accepts but
  also flags `stats.compress_recent_message` for telemetry; the model is
  free to choose, but compressing a just-emitted message is rare.
- Message id corresponds to the very first user message (the system task
  prompt) and `compress.protectUserMessages == true`: rejected as
  `Error::InvalidCompressArgs("user message is protected")`.
- The same message id is referenced in two consecutive compress calls (the
  second after the first succeeded): the second is rejected because the
  message is now inside an active block.
- A message containing only a `tool_call` (no text): legal target; the
  block's summary replaces the call. Pairing of result is preserved by also
  hiding the matching `tool_result` (Section 11.1).

### 6.3 Block bookkeeping

Every block carries the fields below. Fields are immutable after the block
is committed except where noted.

| Field | Type | Mutable | Description |
|-------|------|---------|-------------|
| `block_id` | u32 | no | Allocated by `allocate_block_id`. |
| `run_id` | u32 | no | Allocated by `allocate_run_id` once per compress call. |
| `mode` | enum | no | `range` or `message`. |
| `topic` | string | no | The batch-level topic from the compress call. |
| `batch_topic` | optional string | no | Same as `topic` for range mode; the per-entry topic for message mode. |
| `summary` | string | no | The wrapped summary (after placeholder expansion and protected-content append). |
| `start_id` | string | no | The original `startId`/`messageId` from the call (reference form). |
| `end_id` | string | no | Original `endId` (= `messageId` in message mode). |
| `anchor_message_id` | string | no | Raw id of the anchor message. |
| `compress_message_id` | string | no | Raw id of the assistant message that issued the compress call. |
| `compress_call_id` | optional string | no | Tool call id of the compress invocation. |
| `included_block_ids` | list of u32 | no | All block ids whose summaries were folded into this block (consumed plus their transitive includes). |
| `consumed_block_ids` | list of u32 | no | Block ids that were directly active at compression time and are now deactivated by this block. |
| `parent_block_ids` | list of u32 | yes | Block ids that have consumed this block. Updated when a future compression supersedes this one. |
| `direct_message_ids` | list of string | no | Raw message ids covered directly (excluding messages already inside consumed blocks). |
| `direct_tool_ids` | list of string | no | Tool call ids referenced by `direct_message_ids`. |
| `effective_message_ids` | list of string | no | Union of `direct_message_ids` and the `effective_message_ids` of every consumed block. |
| `effective_tool_ids` | list of string | no | Union of `direct_tool_ids` and the consumed blocks' `effective_tool_ids`. |
| `compressed_tokens` | u64 | no | Sum of token counts over the verbatim content this block replaces (best-effort estimate at commit time). |
| `summary_tokens` | u64 | no | Token count of the wrapped summary string. |
| `duration_ms` | u64 | no | Time spent assembling the block (host-clock, informational). |
| `active` | bool | yes | True while the block is the current representation of its anchor. False after a parent block consumes it. |
| `deactivated_by_user` | bool | yes | True if the user explicitly decompressed via the slash command. |
| `created_at` | i64 ms | no | Wall-clock at commit. |
| `deactivated_at` | optional i64 ms | yes | Set when `active` becomes false. |
| `deactivated_by_block_id` | optional u32 | yes | Set when consumed by a parent block; otherwise `None`. |

#### 6.3.1 Anchor selection rule

`anchor_message_id = first non-consumed raw id in selection`.

If every message in the selection is the anchor of a consumed block, then
the anchor is the very first message id in the selection (so the new block
has a stable visual position).

The anchor is special because it is the message into which the block's
summary is rendered (Section 6.4 — `filter_compressed_ranges`). All other
messages in `direct_message_ids` are removed from the outgoing list.

#### 6.3.2 Protected content append

Before the summary is finalized, the library appends sections for content
the model is configured to never lose verbatim. These appendices are added
in the following order, each guarded by a fenced section:

```
<dcp-protected-user>
... user messages …
</dcp-protected-user>
<dcp-protected-prompt>
... protected system / prompt text …
</dcp-protected-prompt>
<dcp-protected-tools>
... tool outputs whose name is in compress.protectedTools …
</dcp-protected-tools>
<dcp-included-blocks>
... summaries for required blocks not referenced via placeholder …
</dcp-included-blocks>
```

Each section is omitted if it would be empty.

The section for protected user messages is gated by
`compress.protectUserMessages`: when true, every user message inside the
range is appended verbatim (truncated to a configurable per-message cap of
8 KiB at a UTF-8 boundary, with `[truncated]` marker if cut).

#### 6.3.3 Wrapped-summary form

A wrapped summary is the string committed to `block.summary` and rendered
into the outgoing message stream. Its shape:

```
<dcp-block id="b<N>" topic="<escaped topic>">
<dcp-summary>
<actual summary text>
</dcp-summary>
<dcp-protected-user>...</dcp-protected-user>      (optional)
<dcp-protected-prompt>...</dcp-protected-prompt>  (optional)
<dcp-protected-tools>...</dcp-protected-tools>    (optional)
<dcp-included-blocks>...</dcp-included-blocks>    (optional)
</dcp-block>
```

The outer `<dcp-block>` tag is what the apply phase emits; the model sees
this exact format inside its context.

### 6.4 Nesting and consumption

Compression blocks nest naturally. When a new block's range covers any
existing active blocks, those existing blocks are *consumed*: they become
inactive, the new block records them as `consumed_block_ids`, and they
record the new block as a `parent_block_ids` entry.

#### Computing `effective_message_ids` and `effective_tool_ids`

These derived fields satisfy:

```
effective_message_ids(B) = direct_message_ids(B)
                          ∪ ⋃ effective_message_ids(C) for C in consumed_block_ids(B)
effective_tool_ids(B)    = direct_tool_ids(B)
                          ∪ ⋃ effective_tool_ids(C) for C in consumed_block_ids(B)
```

Computed once at commit and frozen on the block. They give the host (and
audit tools) a single field that names every original message and tool call
fully covered by the block, regardless of intermediate consumption.

#### `filter_compressed_ranges`

This is the apply step that turns active blocks into outgoing messages.

Pseudocode:

```
function filter_compressed_ranges(messages, state, config):
    out = []
    skip_ids = {}                       # raw message ids to drop
    for B in active_blocks(state):
        for raw_id in B.direct_message_ids:
            if raw_id != B.anchor_message_id:
                skip_ids.add(raw_id)

    for msg in messages:
        if msg.id in skip_ids:
            continue
        # If this message is the anchor of an active block, replace its
        # text body with the block's wrapped summary.
        block = state.active_by_anchor_message_id.get(msg.id)
        if block is not None:
            msg = render_block_anchor(msg, block, config)
        out.append(msg)
    return out

function render_block_anchor(msg, block, config):
    new_msg = copy(msg)
    # Replace the first text part with the wrapped summary; drop any other
    # text and reasoning parts. Keep tool_call / tool_result parts whose
    # call_id is NOT in block.effective_tool_ids — those belong to other
    # logical operations that happen to share this anchor.
    new_msg.parts = []
    new_msg.parts.append(Text(block.summary))
    for part in msg.parts:
        if part is tool_call/tool_result and part.call_id in block.effective_tool_ids:
            continue
        if part is text or reasoning:
            continue  # already replaced
        new_msg.parts.append(part)
    return new_msg
```

#### `commit_block`

Pseudocode:

```
function commit_block(state, block, plan):
    state.prune.messages.blocks_by_id[block.block_id] = block
    state.prune.messages.active_block_ids.add(block.block_id)
    state.prune.messages.active_by_anchor_message_id[block.anchor_message_id] = block.block_id

    for cid in block.consumed_block_ids:
        consumed = state.prune.messages.blocks_by_id[cid]
        consumed.active = false
        consumed.deactivated_at = now()
        consumed.deactivated_by_block_id = block.block_id
        consumed.parent_block_ids.append(block.block_id)
        state.prune.messages.active_block_ids.remove(cid)
        # The consumed block's anchor entry is now overwritten; it was
        # already set to `cid` and is now reset to `block.block_id` if the
        # anchors coincide, or removed otherwise.
        if state.prune.messages.active_by_anchor_message_id.get(consumed.anchor_message_id) == cid:
            del state.prune.messages.active_by_anchor_message_id[consumed.anchor_message_id]
```

### 6.5 Frontier mechanism

The frontier prevents the library from repeatedly nudging the model to
compress the same range when a previous compression yielded a summary that
is *larger* than the verbatim content it replaced.

#### State

- `state.prune.messages.frontier_message_ref: optional string`. When set,
  any nudge or compress suggestion targeting messages at-or-before this
  reference is suppressed.

#### When to advance

Advance the frontier when, after a compress run completes, **for any single
new block**:

```
block.summary_tokens >= block.compressed_tokens
```

If this condition holds for the block:

1. Set `frontier_message_ref = max(frontier_message_ref, block.end_id)`.
2. Mark the block with an internal flag `oversized = true` (kept only in
   memory; not persisted because the frontier itself is persisted).
3. Decrement `stats.compress_useful` if previously incremented for this
   block; increment `stats.compress_oversized`.
4. The block is **not** rolled back — the model spent effort, the summary
   may still be useful for the next nudge target. But future nudges are
   suppressed up to and including its end.

#### When to skip

Skip emitting a compress nudge when its proposed range would lie entirely
at or before `frontier_message_ref`. Specifically, in
`build_priority_map` (Section 8): a candidate range whose `end_id` is at or
before `frontier_message_ref` is removed from the candidate set.

#### Resetting the frontier

The frontier is reset only when:

- A compaction event is detected (Section 3.3).
- The user invokes a slash command that explicitly clears it
  (`/dcp reset-frontier`).
- A new block is committed whose `summary_tokens < compressed_tokens` and
  whose range extends past `frontier_message_ref` — in this case the
  frontier remains, but it is allowed to be advanced past on the next
  oversized commit beyond it.

#### Edge cases

- All blocks in a single run are oversized: frontier advances to the
  maximum `end_id` among them.
- A block exactly breaks even (`summary_tokens == compressed_tokens`):
  treated as oversized (the compression yielded no benefit).
- The frontier is reached but the model has not been nudged again because
  iteration nudges fired on a different anchor: behavior continues
  unchanged; the frontier silently stays.
- A block past the frontier later becomes consumed by a parent that is
  oversized: frontier advances to the parent's `end_id`.
- The host's tokenizer changes between runs (e.g. config update mid
  session): the frontier comparison uses whichever tokenizer was active at
  commit time, recorded as `block.summary_tokens` and
  `block.compressed_tokens`. No re-counting.


---

## 7. Cache stability

LLM providers commonly cache prompt prefixes byte-for-byte. Once the
library mutates a message in the prefix, the cache misses and cost
increases. The cache-stability subsystem ensures that pruning decisions are
**accumulated** quickly but **applied** only at safe boundaries.

### 7.1 Mode definitions

The active mode is the value of `config.cache_stability_mode`. The three
modes have the following meanings:

| Mode | Description | When the apply phase runs |
|------|-------------|---------------------------|
| `aggressive` | Apply every transform call. Maximum freshness, minimum cache hit rate. Intended for development and debugging. | On every `transform_messages` invocation. |
| `agent_message` | Apply at turn boundaries. Default. Pruning decisions accumulate during tool turns, then flush once the assistant produces a final text response. | On any `transform_messages` invocation where `state.last_message_was_assistant_text == true`. |
| `manual` | Never apply automatically. The host triggers application by calling `force_apply()`. Strategies still compute and accumulate in the pending state for inspection. | Only when `force_apply()` is invoked, or implicitly when `force_apply` flag is set on `apply_pending_manual_trigger`. |

These three values must be exhaustive. `Config::default()` sets the mode to
`agent_message`.

### 7.2 Pending state semantics

Strategies always compute their decisions on every transform call. Whether
those decisions reach the outgoing messages is a separate question, gated
by `should_apply_now`.

When apply is gated off:

1. The strategy still runs and writes into `state.prune.tools` (the canonical
   "what should be pruned" structure). This is intentional: the bookkeeping
   is the source of truth and is needed for telemetry and future invocations.
2. A *snapshot* of the just-added entries is recorded in
   `state.pending_prune`:
   ```
   pending_prune = {
       tool_ids:           [list of newly added call_ids],
       cumulative_tokens:  u64,
       accumulated_at_turn: state.current_turn,
   }
   ```
3. The apply phase consults `state.prune.tools` to decide which parts to
   strip, **but** if cache stability gating is on and we are not at a turn
   boundary, the apply phase is *skipped entirely*: `pruned = valid` is
   returned (as far as content goes), preserving the byte-for-byte prefix
   the cache expects.

Important consequences:

- The cumulative tokens counter is always observable via telemetry, even
  when the apply has not yet run.
- After application, `state.pending_prune` is cleared.
- After a compaction event (Section 3.3), `state.pending_prune` is cleared
  unconditionally because the conversation prefix the cache held no longer
  exists.

#### State fields in detail

| Field | Type | Description |
|-------|------|-------------|
| `state.pending_prune` | optional `PendingPrune` | None means no pending decisions. |
| `state.last_message_was_assistant_text` | bool | Set by the turn-boundary detector (Section 3.2). Read by `should_apply_now`. |
| `state.last_apply_turn` | optional u32 | The `current_turn` value at the most recent successful apply. Used by debug/telemetry. |
| `state.force_apply_requested` | bool | Set when the host calls `force_apply()`. Cleared after the next transform applies. |

### 7.3 Apply triggers

`should_apply_now` is the single function that decides whether the apply
phase runs. It is called once per `transform_messages` invocation, after
turn-boundary detection and before strategy execution.

Pseudocode:

```
function should_apply_now(state, config, messages):
    if state.force_apply_requested:
        return true
    switch config.cache_stability_mode:
        case aggressive:
            return true
        case agent_message:
            return state.last_message_was_assistant_text
        case manual:
            return false
```

After the apply phase actually runs:

```
state.force_apply_requested = false
state.last_apply_turn = state.current_turn
state.pending_prune = None
```

#### Edge cases

- A long sequence of mid-turn transforms accumulates a large
  `pending_prune.cumulative_tokens`. This is informational; nothing
  unusual happens until the turn ends and the apply runs.
- `force_apply()` is called during a tool turn: applies once and resets
  the flag. The next transform may again be mid-turn and will gate normally.
- The mode is changed mid-session (host updates config): the next
  transform reads the new mode. Pending decisions accumulated under the
  old mode are still applied under the new mode's rules.
- A compaction event arrives between strategy run and apply: the apply
  is skipped (because compaction also resets `state.prune.tools` and
  `state.pending_prune`), and the next transform starts fresh.
- An empty input on a turn-boundary moment: apply runs but is a no-op
  because there is nothing to mutate; the turn counter still advances if
  the heuristic detects a turn-end on an earlier message; otherwise it
  remains unchanged.

---

## 8. Nudges

A **nudge** is a short library-controlled instruction inserted into the
outgoing message stream that asks the model to call the compress tool. The
library injects three kinds: context-limit, turn, and iteration. Each type
has its own prompt template, its own threshold logic, and its own anchoring
rule.

Nudges are always rendered at the end of the targeted message's primary
text part, separated by a leading double newline. The injection mode
(`config.compress.injection_mode`) controls *how* the nudge is glued onto
the message:

| Mode | Effect |
|------|--------|
| `append_text` | The nudge is appended to an existing text part. If no text part exists, one is created. |
| `wrap_block` | The nudge is wrapped in `<dcp-nudge kind="…"> … </dcp-nudge>` tags before append. Default. |

### 8.1 Context-limit nudge

Triggered when the total token count of the message stream is above a
fraction of the model's context limit.

#### Threshold computation

Inputs: `total_tokens` (sum of `tokens(serialize(msg))` for every outgoing
message); `max_context_limit` (from
`config.compress.max_context_limit`, possibly model-specific via
`compress.modelMaxLimits`); `min_context_limit` (analogous).

The effective threshold is:

```
threshold = resolve_limit(config.compress.max_context_limit,
                          state.model_context_limit)
```

Where `resolve_limit`:

- If the configured value is a number, return it.
- If the value is a percent string (e.g. `"80%"`), it must be paired with a
  known `state.model_context_limit`. The result is `state.model_context_limit
  * fraction`, rounded down. If the model context limit is unknown, the
  percent form is replaced by a hard-coded fallback of `100_000`.

The nudge fires when `total_tokens > threshold`. The minimum-context-limit
value is the lower bound below which **no** context-limit nudge fires
regardless of any other condition; this prevents nudge spam on tiny
sessions.

#### Frequency

Once the threshold is exceeded, the nudge does not fire on every
transform. Instead it fires every `nudgeFrequency` transform calls
(default `5`). The library tracks a counter
`state.nudges.context_limit_counter`:

- Incremented on every transform after the first time the threshold is
  exceeded.
- When the counter reaches `nudgeFrequency`, render the nudge and reset to
  `0`.
- When `total_tokens` drops below the threshold, the counter resets to `0`
  unconditionally.

#### Anchor selection

The nudge is anchored to the **most recent assistant message** in the
outgoing stream. If no assistant message exists, the nudge is anchored to
the most recent user message.

#### Rendering

The prompt template `prompts.context_limit_nudge` accepts placeholders
`{tokens}` and `{limit}`. The renderer substitutes these with decimal
strings. Default template (illustrative, may be overridden):

```
The conversation has reached {tokens} tokens, near the configured limit
of {limit}. Consider calling the `compress` tool to summarize older
ranges so context remains available for ongoing work.
```

The library never includes more than one context-limit nudge per
transform.

### 8.2 Turn nudge

Triggered when a turn has finished and no compress activity has happened
recently relative to the most recent user request.

#### Trigger logic

For every (user-message, assistant-message) pair where the assistant
message is the turn-end:

- If the pair has not yet had a turn-nudge attached to it, *and* there is
  no active block whose `direct_message_ids` overlap the pair, *and*
  `nudge_force == "soft"`, *and* the pair is older than the most recent
  pair: render a turn nudge.

The turn nudge is only emitted when `nudge_force == "soft"` (the default).
With `nudge_force == "strong"`, the iteration nudge is preferred and the
turn nudge is suppressed.

#### Anchor selection

Turn nudge is anchored to the **assistant** message at the turn end. The
nudge is small and informational; the model is invited to compress the
preceding pair.

#### Rendering

Default template (overridable via `prompts.turn_nudge`):

```
You have just finished a turn. If the preceding exchange contains
information that would not be missed in summary form, consider calling
the `compress` tool with mode "range" to fold it into a block.
```

A pair is marked nudge-attached in `state.nudges.turn_nudged_pairs` (a set
of `(user_id, assistant_id)` tuples). Re-injection on subsequent
transforms is suppressed once attached.

### 8.3 Iteration nudge

Triggered when many messages have accumulated since the most recent user
message — typical of long tool-loop iterations.

#### Count rules

Let `count = number of messages whose role == assistant since (and not
including) the most recent user message`.

The nudge fires when `count > config.compress.iteration_nudge_threshold`
(default `15`). Once fired, the counter is *not* reset; instead, the nudge
re-fires every `nudgeFrequency` further messages until the next user
message arrives. When a new user message arrives, the counter naturally
resets because the count is computed from the latest user message
forward.

#### Anchor selection

Iteration nudge is anchored to the **most recent assistant message**. The
intent is to make the model see the nudge in its immediate context.

#### Rendering

Default template (overridable via `prompts.iteration_nudge`):

```
You have iterated {count} assistant messages without a user prompt.
Older steps in this iteration are unlikely to be needed verbatim.
Consider calling `compress` with mode "range" on the older portion.
```

The renderer substitutes `{count}`. As with other nudges, only one
iteration nudge is emitted per transform.

### 8.4 Nudge priority

The library may compute multiple candidate nudges for a single transform.
Only one is emitted to avoid spamming the model. Priority order, highest
to lowest:

1. Context-limit (only if firing)
2. Iteration (only if firing)
3. Turn (only if firing)

`build_priority_map` consolidates all candidates and selects the single
winner. The selected nudge is mapped to its anchor message id; the apply
phase reads this map and injects accordingly.

`build_priority_map` algorithm:

1. Initialize `winner = None`.
2. If context-limit fires (Section 8.1) and the candidate end is past the
   frontier, set `winner = ContextLimit{tokens, limit, anchor}`.
3. Else if iteration fires (Section 8.3) and past frontier, set
   `winner = Iteration{count, anchor}`.
4. Else if turn fires (Section 8.2) and past frontier, set
   `winner = Turn{anchor}`.
5. Return the map `{anchor_id: winner}` or empty.

Past-the-frontier check: the candidate's anchor message id resolves to a
reference; if the reference is at or before
`state.prune.messages.frontier_message_ref`, the candidate is dropped.


---

## 9. Persistence

The library persists a subset of session state to a host-pluggable storage
backend. The default backend is a file in the user's data directory; an
in-memory backend is also bundled. Custom backends implement the
`StatePersistence` trait.

Persistence is **deferred** by default: state is written only when:

- `save()` is explicitly called.
- An auto-save interval has elapsed since the last write (default disabled;
  enable via `config.persistence.auto_save_seconds`).
- The session is dropped while a save is queued (best-effort flush in
  `Drop`).

### 9.1 Schema V1

The persisted document is a single JSON object. The top-level shape is:

```jsonc
{
    "schema_version": "1",
    "session_name": "<optional human-readable id>",
    "session_id": "<opaque string>",
    "last_updated": "<RFC3339 timestamp>",
    "current_turn": <u32>,
    "frontier_message_ref": "<m####|null>",
    "next_block_id": <u32>,
    "next_run_id": <u32>,
    "next_message_ref": <u32>,
    "stats": <Stats object, see below>,
    "nudges": {
        "context_limit_counter": <u32>,
        "turn_nudged_pairs": [["<user_id>", "<assistant_id>"], …]
    },
    "prune": {
        "tools": { "<call_id>": <u64 tokens saved>, … },
        "messages": {
            "blocks": [<CompressionBlock object>, …],
            "active_block_ids": [<u32>, …]
        }
    },
    "tool_index": {
        "<call_id>": {
            "tool": "<string>",
            "signature": "<string>",
            "status": "<pending|running|completed|error>",
            "turn": <u32>,
            "message_id": "<raw>",
            "result_message_id": "<raw|null>",
            "paths": ["<string>", …],
            "token_count": <u64|null>
        }, …
    },
    "message_id_map": {
        "by_raw_id": { "<raw>": "<m####>", … },
        "by_ref":    { "<m####>": "<raw>", … }
    },
    "compaction": {
        "last_compaction_at": <i64 ms|null>,
        "compactions_observed": <u32>
    }
}
```

The `<CompressionBlock object>` shape mirrors Section 6.3, with all field
names converted to snake_case JSON (Rust convention).

The `<Stats object>` is:

```jsonc
{
    "total_prune_tokens": <u64>,
    "dedup_pruned": <u32>,
    "purge_errors_pruned": <u32>,
    "stale_file_reads_pruned": <u32>,
    "compress_runs": <u32>,
    "compress_blocks_committed": <u32>,
    "compress_oversized": <u32>,
    "compress_useful": <u32>,
    "compactions_observed": <u32>,
    "cache_bust_events": <u32>,
    "orphan_tool_results": <u32>,
    "dropped_invalid": <u32>,
    "invalid_status_transitions": <u32>,
    "normalize_depth_clamped": <u32>,
    "path_null_byte_stripped": <u32>,
    "storage_save_failed": <u32>,
    "persisted_corruption": <u32>
}
```

#### What is **not** persisted

- The full message list (the host owns it).
- Pending in-flight compress runs (allocations not yet committed).
- The transient `pending_prune` (Section 7.2) — by design, restart
  re-derives this.
- Subagent result caches (Section 11.6) — they are derived per session.

#### Field semantics for restoration

| Field | On load |
|-------|---------|
| `schema_version` | Determines migration path. |
| `next_block_id` | Used to initialize allocator; must be greater than any block id in `blocks`. |
| `next_run_id` | Same for runs. |
| `next_message_ref` | Initial value of allocator. |
| `frontier_message_ref` | Restored verbatim. |
| `prune.tools` | Restored. The apply phase will re-strip these tools next transform. |
| `prune.messages.blocks` | Each becomes an entry in `blocks_by_id`. |
| `prune.messages.active_block_ids` | Cross-checked: any id in this list must have `active == true` in its `blocks` entry. Mismatches set `active = false` and increment `stats.persisted_corruption`. |
| `tool_index` | Repopulates `state.tool_parameters`. |
| `message_id_map` | Repopulates `state.message_ids`. |
| `nudges.turn_nudged_pairs` | Restored to the set form. |
| `compaction.last_compaction_at` | Restored. |

### 9.2 Migration rules

Future schema versions will be introduced as `V2`, `V3`, etc. The library
will ship a migrator for every adjacent pair (`v1 → v2`, `v2 → v3`); the
loader chains migrations to upgrade any older document to the current
version.

Migration policy:

| Rule | Description |
|------|-------------|
| Forward-only | The library never writes a downgraded version. |
| Lossless when possible | Every field is preserved; new fields receive sensible defaults. |
| Documented breaking | Any field whose semantics change (rather than additive) is documented in the migration's release notes. |
| Bounded retries | Migration is attempted once; failure produces `Error::PersistenceMigration` and the storage file is renamed to `<name>.migration_failed_<timestamp>.bak` so it isn't lost. |
| Schema-only check first | Before invoking migrators, the loader rejects unknown `schema_version` values higher than the running library's max. |

A no-op migration from V1 to V1 always succeeds and validates required
fields are present.

### 9.3 Atomic write protocol

Writes must be atomic to survive a crash mid-write. The protocol:

1. Compute the target path `target = <store_dir>/sessions/<session_id>.json`.
2. Compute a sibling temp path `tmp = <store_dir>/sessions/<session_id>.json.tmp.<random>`.
3. If a backup is enabled (`config.persistence.keep_backup`, default true)
   and `target` exists, copy `target` to
   `<store_dir>/sessions/<session_id>.json.bak`.
4. Write the serialized JSON to `tmp` and `fsync` the file.
5. Rename `tmp` to `target`. On platforms where rename is atomic across
   files within a directory (POSIX), this completes the operation. On
   non-POSIX platforms, an emulation that deletes then renames is used,
   accepting a slightly larger window for corruption (documented as a
   known limitation).
6. `fsync` the directory containing `target` (POSIX best-practice).

Failures:

- Writing to `tmp` fails: bubble up the error. Target file is unchanged.
- Renaming fails: bubble up the error. Target file is unchanged. The temp
  file is *not* automatically deleted; the next save will reuse a fresh
  random suffix.
- Backup copy fails: log warning, proceed with write.

The resulting on-disk layout for the default file backend:

```
$XDG_DATA_HOME/dynamic_context_pruning/
└── sessions/
    ├── <session_id>.json
    ├── <session_id>.json.bak           (if keep_backup)
    └── <session_id>.json.tmp.<random>  (only between steps 4 and 5)
```

If `XDG_DATA_HOME` is unset, the library falls back to
`~/.local/share/dynamic_context_pruning/sessions/`.

#### Edge cases

- The directory does not exist: it is created with mode `0700` on first
  write.
- Concurrent writers (two `ContextPruner` instances for the same session
  id, possibly in different processes): undefined; the second writer's
  rename overwrites the first's. The library does not file-lock by
  default. A future feature flag may enable advisory locking.
- The disk is full: write fails; backup is preserved.
- A power loss between fsync and rename: the target retains its previous
  content; the temp file is left as garbage and is cleaned up on the next
  successful save (any `*.tmp.*` siblings older than the current target
  are removed).
- A power loss between rename and directory fsync: extremely rare; the
  rename has already taken effect on most filesystems.


---

## 10. Configuration

The library is configured via a JSONC document. Every field has a documented
default; missing fields take their default. Unknown fields produce a
warning but do not fail loading (forward-compatibility).

### 10.1 Cascade order

Configuration is resolved by merging documents in the following order, each
later layer overriding earlier ones:

1. **Built-in defaults** — compiled into the library.
2. **Global config** — `$XDG_CONFIG_HOME/dynamic_context_pruning/config.jsonc`
   (fallback `~/.config/dynamic_context_pruning/config.jsonc`).
3. **Custom directory** — `$DCP_CONFIG_DIR/config.jsonc` if the env var is
   set and points to a readable file.
4. **Project config** — the file `.dynamic_context_pruning/config.jsonc`
   located in the host's working directory or any ancestor up to a
   filesystem root or a marker file (`.git`, `Cargo.toml`,
   `pyproject.toml`, `package.json`).
5. **Programmatic overrides** — any settings passed via
   `Config::with_overrides` or `ContextPrunerBuilder::*`.

Merge rules:

- Object fields are deep-merged (later object replaces matching keys; other
  keys retained).
- Array fields are *replaced wholesale* (later array replaces earlier).
  Lists like `protected_file_patterns` are not concatenated.
- Scalar fields are replaced.
- The boolean `enabled = false` at any level disables the feature for all
  later levels unless a later level explicitly sets `enabled = true`.

### 10.2 Field semantics

The table below enumerates every configurable field, its type, default,
allowed range, and behavior. The JSONC keys use camelCase to match common
host configuration conventions; the in-memory Rust representation may use
snake_case.

#### Top-level fields

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `enabled` | bool | `true` | — | Master switch. When `false`, `transform_messages` is a pass-through. |
| `debug` | bool | `false` | — | Enables verbose telemetry and debug log output. |
| `cacheStabilityMode` | enum string | `"agent_message"` | `"aggressive"`, `"agent_message"`, `"manual"` | Section 7.1. |
| `injectionMode` | enum string | `"wrap_block"` | `"append_text"`, `"wrap_block"` | How nudges are glued onto messages (Section 8). |
| `protectedFilePatterns` | string[] | `[]` | glob patterns | Sections 5.1, 5.3. |

#### `notification`

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `notification.level` | enum string | `"detailed"` | `"off"`, `"minimal"`, `"detailed"` | Verbosity for host-side notifications. |
| `notification.kind` | enum string | `"chat"` | `"chat"`, `"toast"` | Where to render notifications. |

#### `manualMode`

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `manualMode.enabled` | bool | `false` | — | If true, the host is expected to drive everything via slash commands. |
| `manualMode.automaticStrategies` | bool | `true` | — | If false (and `manualMode.enabled == true`), strategies do not run automatically. |

#### `turnProtection`

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `turnProtection.enabled` | bool | `false` | — | Reserved for future use; currently has no effect on logic. |
| `turnProtection.turns` | u32 | `4` | `0..=100` | Number of recent turns shielded from compression suggestions when enabled. |

#### `compress`

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `compress.mode` | enum string | `"range"` | `"range"`, `"message"` | The active compress tool mode. |
| `compress.permission` | enum string | `"allow"` | `"ask"`, `"allow"`, `"deny"` | Whether the host should prompt before compress runs. `"deny"` rejects all compress calls. `"ask"` causes the library to expose a permission gate; the host is expected to confirm and then call `set_compress_permission(allow)`. |
| `compress.showCompression` | bool | `false` | — | When true, the rendered block summaries include a wrapping marker comment for human inspection. |
| `compress.summaryBuffer` | bool | `true` | — | When true, summaries are buffered through the prompt-engineering append step; when false, the model's exact summary is committed unchanged. |
| `compress.maxContextLimit` | number\|string | `100000` | `>= 1000` or `"X%"` with `0% < X <= 100%` | Section 8.1. |
| `compress.minContextLimit` | number\|string | `50000` | `>= 1000` or `"X%"` | Section 8.1. |
| `compress.modelMaxLimits` | object | `{}` | per-model overrides | Map from model id to a number or `"X%"` string. |
| `compress.modelMinLimits` | object | `{}` | per-model overrides | Same shape as above. |
| `compress.nudgeFrequency` | u32 | `5` | `1..=100` | Section 8.1, 8.3. |
| `compress.iterationNudgeThreshold` | u32 | `15` | `2..=100` | Section 8.3. |
| `compress.nudgeForce` | enum string | `"soft"` | `"strong"`, `"soft"` | Section 8.2. |
| `compress.protectedTools` | string[] | `["task", "skill"]` | tool names | Tool names whose verbatim output is appended in `<dcp-protected-tools>` when their range is compressed. |
| `compress.protectTags` | bool | `false` | — | When true, text inside `<dcp-protected> … </dcp-protected>` tags is preserved verbatim across compression. |
| `compress.protectUserMessages` | bool | `false` | — | Section 6.3.2. |
| `compress.maxSummaryChars` | u32 | `32768` | `1024..=262144` | Per-entry summary length cap. |

#### `strategies`

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `strategies.deduplication.enabled` | bool | `true` | — | Section 5.1. |
| `strategies.deduplication.protectedTools` | string[] | `[]` | tool names | Section 5.1. |
| `strategies.purgeErrors.enabled` | bool | `true` | — | Section 5.2. |
| `strategies.purgeErrors.turns` | u32 | `4` | `1..=100` | Section 5.2 threshold. |
| `strategies.purgeErrors.protectedTools` | string[] | `[]` | tool names | Section 5.2. |
| `strategies.staleFileReads.enabled` | bool | `true` | — | Section 5.3. |
| `strategies.staleFileReads.protectedTools` | string[] | `[]` | tool names | Section 5.3. |
| `strategies.staleFileReads.trackedTools` | string[] | `["read", "write", "edit", "multiedit"]` | tool names | Section 4.6. |

#### `commands`

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `commands.enabled` | bool | `true` | — | Whether the slash-command surface (`/dcp …`) is exposed to the host. |
| `commands.protectedTools` | string[] | `[]` | tool names | Used by `/dcp sweep` to pre-protect tools the user does not want touched. |

#### `experimental`

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `experimental.allowSubagents` | bool | `false` | — | Section 11.6. |
| `experimental.customPrompts` | bool | `false` | — | When true, the host may override default nudge prompts via `Prompts`. |

#### `persistence`

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `persistence.enabled` | bool | `true` | — | Master switch for storage backend. |
| `persistence.autoSaveSeconds` | u32 | `0` | `0..=3600` | When non-zero, the library saves automatically at most this often. `0` means save only on explicit `save()`. |
| `persistence.keepBackup` | bool | `true` | — | Section 9.3 step 3. |
| `persistence.path` | optional string | `null` | absolute path | Override storage directory. When `null`, defaults to XDG path. |

#### `tokenizer`

| Field | Type | Default | Range | Description |
|-------|------|---------|-------|-------------|
| `tokenizer.kind` | enum string | `"chars_div_4"` | `"chars_div_4"`, `"tiktoken"`, `"hf"`, `"claude"`, `"custom"` | Selects the bundled tokenizer or a custom one (configured programmatically). |
| `tokenizer.model` | optional string | `null` | model id or path | Required for `tiktoken`, `hf`, `claude`. Ignored for `chars_div_4`. |
| `tokenizer.imageTokens` | u32 | `1500` | `1..=10000` | Per-image token cost. |

### 10.3 Validation rules

Validation runs after the cascade is fully resolved and before the
`ContextPruner` is constructed. Each rule below lists the failure outcome.

| Rule | Failure |
|------|---------|
| `cacheStabilityMode` is one of the three known values | `Error::Config("invalid cacheStabilityMode")` |
| `injectionMode` is one of the two known values | `Error::Config("invalid injectionMode")` |
| `compress.mode` is one of the two known values | `Error::Config("invalid compress.mode")` |
| `compress.permission` is one of the three known values | `Error::Config("invalid compress.permission")` |
| `compress.maxContextLimit` numeric form is `>= 1000` | `Error::Config("maxContextLimit too small")` |
| `compress.maxContextLimit` percent form is `> 0%` and `<= 100%` | `Error::Config("invalid percent")` |
| `compress.minContextLimit` resolves below `maxContextLimit` for any model | `Error::Config("minContextLimit must be < maxContextLimit")` |
| `compress.nudgeFrequency` is in `1..=100` | `Error::Config(...)` |
| `compress.iterationNudgeThreshold` is in `2..=100` | `Error::Config(...)` |
| `compress.maxSummaryChars` is in `1024..=262144` | `Error::Config(...)` |
| `strategies.purgeErrors.turns` is in `1..=100` | `Error::Config(...)` |
| `strategies.staleFileReads.trackedTools` is non-empty when `staleFileReads.enabled == true` | `Error::Config("trackedTools cannot be empty when enabled")` |
| `protectedFilePatterns` glob compiles | `Error::Config("invalid glob: …")` |
| `tokenizer.kind == "custom"` requires a programmatic Tokenizer instance | `Error::Config("custom tokenizer requires builder injection")` |
| `tokenizer.imageTokens` in `1..=10000` | `Error::Config(...)` |
| `persistence.autoSaveSeconds` in `0..=3600` | `Error::Config(...)` |
| `notification.level` is one of three known values | `Error::Config(...)` |
| `notification.kind` is one of two known values | `Error::Config(...)` |
| `experimental.customPrompts == false` and a programmatic prompt override was supplied | warning only; overrides are ignored. |

Unknown top-level keys produce a warning logged to stderr (or a logger
trait if installed) but never block startup. The validator records all
warnings in a returned `ConfigDiagnostics` value alongside the resolved
config.


---

## 11. Edge cases and invariants

This section enumerates the global invariants that every conformant
implementation must preserve, along with related edge-case behaviors. Test
fixtures (Section 12) verify each invariant.

### 11.1 Tool call/result pairing must be preserved

**Invariant**: in any output of `transform_messages`, every `tool_result`
part has a corresponding `tool_call` part with the same `call_id` earlier
in the stream, and every `tool_call` either has a matching `tool_result`
later or is the most recent (still-pending) call.

**Implication**: pruning a `tool_call` must also prune its matching
`tool_result`, and vice versa. The apply phase coordinates this via the
shared key `call_id` in `state.prune.tools`: when an entry is in the prune
map, both the call part and the result part are stripped.

**Special cases**:

- *Purge-errors mode*: replaces input *content* but keeps the part
  envelope. The pairing between call and result is therefore preserved
  even though the input has been gutted.
- *Compression mode*: when a tool call/result pair is fully covered by a
  block's `effective_tool_ids`, both parts are removed in
  `render_block_anchor` (Section 6.4).
- *Mid-flight call*: a call without a result (because the result has not
  arrived yet) is never pruned.

**Test verification**: the property test `prop_tool_pairing_preserved`
asserts the invariant on every output across thousands of random inputs
(Section 12, T-PR-1).

### 11.2 Block IDs are monotonic

**Invariant**: for any two blocks `A` and `B` committed in the same
session, if `A.created_at <= B.created_at`, then `A.block_id <
B.block_id`. Block ids are never reused, even after deactivation.

**Allocation**: `allocate_block_id` reads, captures, and increments
`state.prune.messages.next_block_id` atomically (Section 2.4). On
persistence reload, `next_block_id` is set to `max(existing) + 1`.

**Edge cases**:

- A compress run that fails partway after allocating a block id: the id
  remains "burned"; the next successful allocation skips past it. This is
  preferable to id reuse, which would cause persisted-state ambiguity.
- Two compress runs interleaved (impossible under the sync API but
  conceivable under async): the API serializes through `&mut self`, so
  this case cannot occur in conformant usage.
- A block id near `u32::MAX`: allocation continues; saturation produces
  `Error::AllocatorExhausted`. In practice this is unreachable.

### 11.3 UTF-8 boundary safety

**Invariant**: every string the library emits — block summaries, nudges,
placeholder expansions, persisted JSON — is valid UTF-8 with no
mid-codepoint truncation.

**Operations that must respect boundaries**:

- Truncating long file paths (Section 4.6) at 4096 bytes.
- Truncating protected user messages (Section 6.3.2) at 8 KiB.
- Truncating per-entry summaries to `compress.maxSummaryChars` characters.
- Slicing for any debug log line.

**Implementation hint** (not normative): never truncate at a byte index
without verifying that the index is at a UTF-8 codepoint boundary; back up
to the previous boundary if necessary.

**Edge cases**:

- Multi-codepoint grapheme clusters (e.g. emoji with skin-tone modifiers):
  the library truncates at codepoint boundaries, not grapheme boundaries.
  Truncation may split a grapheme; this is documented.
- Input with stray surrogate halves (invalid UTF-16): such input must
  never occur (canonical messages are validated UTF-8). If it somehow
  does, the validator drops the message.
- BOM (byte-order mark) at the start of a string: kept verbatim.

### 11.4 Idempotent rebuild guarantee

**Invariant**: `rebuild_from_messages(messages, persisted_blocks, config)`
produces a `SessionState` that is functionally equivalent to running the
full session from scratch. Specifically:

```
rebuild(messages, blocks, config).compute_pruning_decisions(messages)
  == original.compute_pruning_decisions(messages)
```

where `compute_pruning_decisions` returns the set of `(call_id, decision)`
that the strategies would emit on the next transform.

**Implication**: a process crash plus restart followed by `transform_
messages` yields identical pruning behavior, modulo timestamps. This means
no pruning decision depends on hidden state that is not derivable from
`(messages, persisted_blocks, config)`.

**State that must therefore not be hidden**:

- Tool parameter normalization: derived from `messages`.
- Signatures: derived from `messages`.
- Active block set: derived from `persisted_blocks`.
- Frontier: persisted (Section 9.1).
- Turn counter: derived by re-running turn-boundary detection over
  `messages`.

**Edge cases**:

- A turn that is mid-compaction at the moment of crash: rebuild treats
  the post-crash messages as a new session prefix; if the host did not
  persist the frontier, the rebuilt session may re-suggest already-tried
  compressions. The host is encouraged to call `save()` after every
  compress run to avoid this.
- Messages whose order cannot be reconstructed (the library relies on
  list order): if the host persists in a different order, behavior is
  undefined. The list passed to `rebuild` must be in canonical
  conversation order.

### 11.5 Non-overlapping ranges in compression

**Invariant**: within a single compress invocation (and across invocations
that have not yet committed), the ranges submitted in `args.content` must
have non-overlapping resolved selections. Two ranges overlap if their
selections share any non-anchor message id.

**Detection** (`validate_non_overlapping`):

1. Build a set of all `direct_message_ids` across plans.
2. If the cumulative count is less than the size of the union, at least
   one id is repeated; identify pairs and emit
   `Error::RangeOverlap("ranges X and Y both cover m####")`.

**Cross-invocation**: a range whose selection includes a message that is
already inside an active block is *not* an overlap in the validation
sense — the existing block is consumed (Section 6.1). True overlap is
only a within-invocation concept.

**Edge cases**:

- Two ranges that share only an active block (no extra messages): legal
  if the block is the only overlap; rejected if the model meant for both
  ranges to consume the same block (semantics ambiguous). The library
  rejects with `Error::RangeOverlap`.
- A range that sandwiches another range entirely: rejected; this is a
  superset/subset, which would force the inner range to be useless.
- An empty selection (resolved range contains zero messages): rejected
  with `Error::InvalidCompressArgs("empty range")`.

### 11.6 Subagent isolation

**Invariant**: a subagent session — a child `ContextPruner` instantiated
to handle a delegated task — does not share state with its parent. The
parent's `state.prune` and `state.message_ids` are not visible to the
child, and vice versa.

**Subagent detection**: `state.is_subagent` is set to `true` by an
explicit constructor flag (`ContextPrunerBuilder::subagent(true)`). The
library has no implicit detection.

**Behavior under `experimental.allowSubagents == false`** (default):

- A subagent's `transform_messages` is a pass-through. No strategies run,
  no compression is offered. The model sees raw messages.

**Behavior under `experimental.allowSubagents == true`**:

- The subagent runs the full pipeline like a top-level session.
- The subagent's persisted state, if any, lives at
  `<store_dir>/sessions/sub_<parent_session_id>_<subagent_session_id>.json`.

**Folding subagent results back into the parent**: the parent calls
`fold_subagent(subagent_messages)` after the subagent finishes. The
library:

1. Constructs a single `Message` whose `role == user` and whose `parts`
   contain a single `text` part of the form:
   ```
   <dcp-subagent id="<subagent_session_id>">
   <dcp-summary>... last assistant text from subagent ...</dcp-summary>
   <dcp-protected-tools>... see below ...</dcp-protected-tools>
   </dcp-subagent>
   ```
2. The protected-tools section contains any tool results from the
   subagent whose tool name is in
   `compress.protectedTools` (e.g. `task` results). Other tool noise is
   omitted.
3. The parent stores the subagent's session id in
   `state.subagent_result_cache` so subsequent transforms can re-emit
   the folded message without recomputing.

**Edge cases**:

- A subagent that itself spawns subagents: legal; nesting is unbounded
  in principle but the library does not synthesize ids beyond the
  parent/child relationship described above.
- Folding the same subagent twice: the second call is a no-op; the
  cached folded message is reused.
- Folding a subagent before it has produced any assistant text: the
  summary is empty (`"<no assistant text>"`).

### 11.7 Reference exhaustion

**Invariant**: message reference allocator has hard upper bound `m9999`.
Block id allocator's bound is `u32::MAX` (effectively unbounded).

**Behavior at exhaustion**:

- `allocate_next_message_ref` returns `Error::MessageRefExhausted`. The
  library declines to allocate further refs for the session. Existing
  refs continue to work, and the affected `transform_messages` call
  returns the error (the host is expected to start a fresh session).

**Edge case**: a session that produces 9 999 messages without any
compression is contrived; in practice nudges and compression keep the
working set well below this bound.

### 11.8 Determinism

**Invariant**: given identical inputs (messages, config, prior persisted
state) and a deterministic tokenizer, `transform_messages` produces
byte-identical output across processes and platforms.

**Sources of nondeterminism the library forbids**:

- Iteration order of unordered hash containers when used to drive output.
  Wherever output order is observable, the library uses stable insertion
  order or sorts.
- Wall-clock-driven decisions on the hot path (timestamps are recorded
  but never affect strategy decisions; only telemetry).
- Random number generators (the only RNG use is the `tmp.<random>`
  suffix in atomic write, which is observation-irrelevant).

**Tokenizer caveat**: the bundled `chars_div_4` tokenizer is fully
deterministic. Third-party tokenizers should be too, but the library
cannot enforce this; it is documented as a host responsibility.

### 11.9 Protected-content immutability

**Invariant**: any text matched by `compress.protectTags` (when enabled)
or content in protected user messages (when enabled) is preserved
verbatim across all compress operations. The library never paraphrases or
truncates protected content beyond the documented per-section caps.

**Edge cases**:

- Nested protected tags (`<dcp-protected> A <dcp-protected> B </dcp-
  protected> C </dcp-protected>`): the outer span is treated as a single
  protected region; inner tags are preserved verbatim inside.
- Protected content with broken tags (opening without closing): the
  malformed span is not protected; the surrounding content is
  compressed normally. Telemetry counter
  `stats.malformed_protect_tags` is incremented.
- Protected content larger than `maxSummaryChars` for a single block: the
  library still includes it, growing the wrapped summary beyond the cap;
  the cap applies only to the summary text *before* protected appendices.

### 11.10 Reentrancy

The library is not reentrant. A `ContextPruner` instance must not be
called from within one of its own callbacks (e.g. tokenizer trait
implementations must not call back into the pruner). Implementations
should not assume `transform_messages` may be re-invoked inside a
`StatePersistence::save` call.

---

## 12. Test fixtures

The minimum set of test fixtures every implementation must pass. Each
fixture is identified by an id (e.g. `T-PR-1`); a conformance test suite
shipped with the library uses these ids as test names. The fixture set is
the lower bound; implementations are encouraged to add more.

Coverage matrix:

| Id | Area | Description | Verifies |
|----|------|-------------|----------|
| T-IR-1 | Canonical IR | A message with all five part variants round-trips through the IR validator with no changes. | Section 2.2 |
| T-IR-2 | Canonical IR | A user message containing a tool_call is rejected (role consistency). | Section 2.5 |
| T-IR-3 | Canonical IR | Two messages with identical id are deduplicated to one (later dropped). | Section 2.5 |
| T-IR-4 | Canonical IR | A tool_result whose call_id has no preceding tool_call is dropped. | Section 2.5, 4.2 |
| T-IR-5 | References | First non-ignored message receives `m0001`. The 9999th message receives `m9999`. The 10000th yields an error. | Section 2.4, 11.7 |
| T-LC-1 | Lifecycle | Empty input list followed by non-empty input progresses through session-start. | Section 3.1 |
| T-LC-2 | Lifecycle | Single user→assistant pair with text terminates the turn; `current_turn` advances by 1. | Section 3.2 |
| T-LC-3 | Lifecycle | An assistant message with both text and an unanswered tool_call does *not* terminate the turn. | Section 3.2 |
| T-LC-4 | Compaction | Replacing the entire message list with new ids resets references and clears tool caches; `compactions_observed` increments. | Section 3.3 |
| T-LC-5 | Compaction | Replacing only the oldest 10% of messages with new ids does not trigger compaction (recent-three heuristic). | Section 3.3 |
| T-TT-1 | Tool tracking | Two calls of `read("foo.rs")` in different turns yield identical signatures. | Section 4.5 |
| T-TT-2 | Tool tracking | A call with `params = {"a": 1, "b": 2}` and another with `{"b": 2, "a": 1}` yield identical signatures (key sort). | Section 4.4, 4.5 |
| T-TT-3 | Tool tracking | A `tool_result` with `status: error` flips the entry's status; `entry.status = error`. | Section 4.3 |
| T-TT-4 | Tool tracking | An attempted transition `completed → error` is rejected; `invalid_status_transitions` increments. | Section 4.3 |
| T-TT-5 | Path extraction | `read({"path": "src/main.rs"})` extracts `src/main.rs`. | Section 4.6 |
| T-TT-6 | Path extraction | `multiedit({"edits": [{"path": "a.rs"}, {"path": "b.rs"}]})` extracts both paths. | Section 4.6 |
| T-TT-7 | Path extraction | `bash({"cmd": "ls foo.rs"})` extracts no paths (bash not in `tracked_tools`). | Section 4.6 |
| T-PR-1 | Strategy: dedup | Three identical `read("foo.rs")` calls; the first two are pruned, the third remains. | Section 5.1 |
| T-PR-2 | Strategy: dedup | A protected tool name is never pruned regardless of duplicates. | Section 5.1 |
| T-PR-3 | Strategy: dedup | Errored tool calls are not grouped (only `completed` are). | Section 5.1 edge |
| T-PR-4 | Strategy: dedup | Path covered by `protected_file_patterns` causes the call to be skipped. | Section 5.1 |
| T-PE-1 | Strategy: purge_errors | An errored call older than `turns=4` has its input replaced with the placeholder; result kept. | Section 5.2 |
| T-PE-2 | Strategy: purge_errors | An errored call exactly at the threshold (`age == turns`) is purged. | Section 5.2 (`>=`) |
| T-PE-3 | Strategy: purge_errors | An errored call protected by tool name is never purged. | Section 5.2 |
| T-PE-4 | Strategy: purge_errors | `turns = 0` is clamped to `1`; current-turn errors not purged. | Section 5.2 |
| T-SF-1 | Strategy: stale_file_reads | Three reads of `foo.rs` produce two prunes; the latest is kept. | Section 5.3 |
| T-SF-2 | Strategy: stale_file_reads | A `write(foo.rs)` after `read(foo.rs)` causes the read to be the older and pruned. | Section 5.3 |
| T-SF-3 | Strategy: stale_file_reads | A `multiedit` referencing two paths counts as latest for both. | Section 5.3 |
| T-SF-4 | Strategy: stale_file_reads | `protected_file_patterns` exempts the path from this strategy. | Section 5.3 |
| T-PI-1 | Pipeline | All three strategies run in order in the same transform. | Section 5.4 |
| T-PI-2 | Pipeline | Subagent with `allowSubagents=false` returns the input untouched. | Section 5.4, 11.6 |
| T-PI-3 | Pipeline | Apply phase removes both call and result for a deduped call. | Section 11.1 |
| T-CO-1 | Compress: range | A single-range compression replaces the range with the wrapped summary on the anchor message; other messages are removed. | Section 6.1, 6.4 |
| T-CO-2 | Compress: range | Two non-overlapping ranges in one call commit two blocks under one run id. | Section 6.1 |
| T-CO-3 | Compress: range | Two overlapping ranges produce `Error::RangeOverlap`. | Section 6.1, 11.5 |
| T-CO-4 | Compress: range | A range that fully contains an active block consumes that block; consumed block becomes inactive; new block records `consumed_block_ids`. | Section 6.4 |
| T-CO-5 | Compress: range | A range that partially overlaps an active block is rejected. | Section 6.1 edge |
| T-CO-6 | Compress: placeholders | A summary with `{{block:b3}}` validates and expands. | Section 6.1 |
| T-CO-7 | Compress: placeholders | A required block not referenced is auto-appended at the end. | Section 6.1 |
| T-CO-8 | Compress: placeholders | A placeholder for a block not in the range fails validation. | Section 6.1 |
| T-CO-9 | Compress: message | A single message-mode entry creates a single-message block with empty consumed list. | Section 6.2 |
| T-CO-10 | Compress: message | Duplicate `messageId` in the same call yields error. | Section 6.2 |
| T-CO-11 | Compress: message | A message currently inside an active block cannot be compressed in message mode. | Section 6.2 |
| T-CO-12 | Compress: bookkeeping | `effective_message_ids` of a parent block equals `direct ∪ effective(child)`. | Section 6.4 |
| T-FR-1 | Frontier | A block with `summary_tokens >= compressed_tokens` advances `frontier_message_ref` to its `end_id`. | Section 6.5 |
| T-FR-2 | Frontier | A nudge candidate at-or-before the frontier is suppressed. | Section 6.5, 8.4 |
| T-FR-3 | Frontier | A compaction event clears the frontier. | Section 6.5 |
| T-CS-1 | Cache: aggressive | Apply runs on every transform. | Section 7.1 |
| T-CS-2 | Cache: agent_message | Apply skipped during a tool turn; runs on assistant text turn-end. | Section 7.1, 7.3 |
| T-CS-3 | Cache: manual | Apply does not run automatically; `force_apply()` triggers one application. | Section 7.1, 7.3 |
| T-CS-4 | Cache: pending | Strategies write to `state.prune.tools` even when apply gated; `state.pending_prune` reflects accumulation. | Section 7.2 |
| T-CS-5 | Cache: compaction | A compaction event clears `state.pending_prune` and `state.prune.tools`. | Section 7.2, 3.3 |
| T-NU-1 | Nudge: context-limit | Token count above threshold fires the nudge after `nudgeFrequency` calls. | Section 8.1 |
| T-NU-2 | Nudge: context-limit | Token count drops below threshold and counter resets. | Section 8.1 |
| T-NU-3 | Nudge: context-limit | Percent-form (`"80%"`) with known model context limit resolves correctly. | Section 8.1 |
| T-NU-4 | Nudge: context-limit | Percent-form with unknown model context limit falls back to `100_000`. | Section 8.1 |
| T-NU-5 | Nudge: turn | A pair without a block attached emits a turn nudge once; subsequent transforms do not re-emit. | Section 8.2 |
| T-NU-6 | Nudge: turn | `nudge_force = "strong"` suppresses turn nudges. | Section 8.2 |
| T-NU-7 | Nudge: iteration | More than `iterationNudgeThreshold` assistant messages since the last user message fires the nudge. | Section 8.3 |
| T-NU-8 | Nudge: iteration | A new user message resets the count. | Section 8.3 |
| T-NU-9 | Nudge: priority | Both context-limit and iteration eligible — only context-limit emitted. | Section 8.4 |
| T-PE-S1 | Persistence | A round trip `save()` → load yields a state functionally equivalent to the original (idempotent rebuild). | Section 9.1, 11.4 |
| T-PE-S2 | Persistence | Schema with unknown future version is rejected with `PersistenceVersionTooNew`. | Section 9.2 |
| T-PE-S3 | Persistence | A crash mid-write (simulated by truncating the temp file) leaves the previous file intact and recoverable. | Section 9.3 |
| T-PE-S4 | Persistence | Backup file (`*.json.bak`) exists after an overwrite. | Section 9.3 |
| T-PE-S5 | Persistence | `persistence.enabled = false` makes `save()` a no-op. | Section 10.2 |
| T-CF-1 | Config | Cascade order: project beats global beats default. | Section 10.1 |
| T-CF-2 | Config | An invalid `cacheStabilityMode` value rejects construction with `Error::Config`. | Section 10.3 |
| T-CF-3 | Config | An unknown top-level key produces a warning, not an error. | Section 10.3 |
| T-CF-4 | Config | `compress.minContextLimit >= maxContextLimit` rejects validation. | Section 10.3 |
| T-CF-5 | Config | `protectedFilePatterns` with a syntactically invalid glob rejects validation. | Section 10.3 |
| T-CF-6 | Config | Per-model max limit overrides the global one when the model id matches. | Section 10.2 |
| T-IN-1 | Invariant | Property test: tool call/result pairing preserved across 1000 random message streams. | Section 11.1 |
| T-IN-2 | Invariant | Property test: block ids are strictly increasing across 1000 random compress sequences. | Section 11.2 |
| T-IN-3 | Invariant | Truncation of a 10 000-byte multi-language string at 4096 bytes preserves UTF-8. | Section 11.3 |
| T-IN-4 | Invariant | Property test: idempotent rebuild from `(messages, blocks, config)`. | Section 11.4 |
| T-IN-5 | Invariant | Property test: non-overlapping ranges enforced across all valid inputs. | Section 11.5 |
| T-IN-6 | Invariant | Subagent with `allowSubagents=false`: state mutations are zero. | Section 11.6 |
| T-IN-7 | Invariant | Determinism: two processes given identical inputs produce identical outputs. | Section 11.8 |
| T-EX-1 | Edge: nesting | Three-level nesting: A consumed by B, B consumed by C; `effective_message_ids(C)` covers all original ids. | Section 6.4 |
| T-EX-2 | Edge: empty session | `transform_messages([])` returns `[]` and does not crash. | Section 3.1 |
| T-EX-3 | Edge: very large summary | A summary at exactly `maxSummaryChars` succeeds; one byte longer fails. | Section 10.3 |
| T-EX-4 | Edge: image counting | Image part contributes exactly `tokenizer.imageTokens` tokens. | Section 10.2 |
| T-EX-5 | Edge: mixed-mode reject | A range-mode tool schema cannot be invoked when `compress.mode = "message"`. | Section 6 |

### 12.1 Property test schemas

The property tests above use the following abstract input generators
(each implementation supplies its own concrete generator):

| Generator | Produces |
|-----------|----------|
| `arb_message` | A canonical message with random valid parts. |
| `arb_messages(n)` | An ordered list of length up to `n` such that tool pairing rules hold. |
| `arb_compress_args` | A valid `CompressArgs` against a given message stream. |
| `arb_session` | A complete session: messages + blocks + valid config. |
| `arb_config` | A valid configuration object exercising every field's range. |

### 12.2 Telemetry assertions

In addition to behavioral assertions, certain fixtures verify telemetry
counters increment correctly:

| Counter | Test | Required behavior |
|---------|------|------------------|
| `dedup_pruned` | T-PR-1 | Increments by exactly 2. |
| `purge_errors_pruned` | T-PE-1 | Increments by exactly 1. |
| `stale_file_reads_pruned` | T-SF-1 | Increments by exactly 2. |
| `compress_blocks_committed` | T-CO-2 | Increments by 2 in one call. |
| `compress_runs` | T-CO-2 | Increments by 1. |
| `compress_oversized` | T-FR-1 | Increments by 1. |
| `compactions_observed` | T-LC-4 | Increments by 1. |
| `cache_bust_events` | T-CS-1 with apply mid-turn | Increments on each mid-turn apply. |
| `dropped_invalid` | T-IR-2 | Increments by 1. |
| `orphan_tool_results` | T-IR-4 | Increments by 1. |

### 12.3 Conformance grade

An implementation is **conformant at level 1** if all `T-IR-*`, `T-LC-*`,
`T-TT-*`, `T-PR-*`, `T-PE-*`, `T-SF-*`, `T-PI-*` fixtures pass.

It is **conformant at level 2** if it additionally passes all `T-CO-*`,
`T-FR-*`, `T-CS-*`, `T-NU-*` fixtures.

It is **fully conformant (level 3)** if it additionally passes all
`T-PE-S*`, `T-CF-*`, `T-IN-*`, `T-EX-*` fixtures.

The published Rust implementation targets level 3 from initial release.

---

## End of specification
