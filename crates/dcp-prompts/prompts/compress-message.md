Compress a single earlier message in place, replacing its body with a
summary while keeping its position in the stream.

Required arguments:
- `message_id`: message reference (e.g. `m0042`) of the message to fold.
- `topic`: short label for the fold.
- `summary`: replacement text.

Use this for individual oversized tool outputs or pasted logs whose
detail is no longer needed. Preserve any identifiers (paths, ids,
errors) that future steps may look up.
