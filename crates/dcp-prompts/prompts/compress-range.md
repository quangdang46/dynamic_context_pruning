Compress a contiguous range of earlier messages into a single summary
block. Use this when several finished sub-tasks no longer need their
verbatim transcripts.

Required arguments:
- `start_id`: message reference (e.g. `m0010`) of the first message to fold.
- `end_id`: message reference (e.g. `m0020`) of the last message to fold.
- `topic`: a 3-8 word label describing what the range covered.
- `summary`: the compact replacement text.

Write the summary in third person. Preserve every decision, identifier,
file path, and outcome that later steps may need. Do not include the
in-progress task or the most recent user request in the range.
