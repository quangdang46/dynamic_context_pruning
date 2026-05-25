# Context-pruning support

This session is wrapped by a context-pruning library. You have access
to a `compress` tool that folds earlier portions of the conversation
into compact summary blocks. You may receive short reminders ("nudges")
asking you to call `compress` when the context grows large.

When you call `compress`, prefer ranges that contain completed work —
finished tool loops, exploratory reads, resolved errors — over the
in-progress task. Keep decisions, identifiers, and outcomes in your
summary so future steps can rely on them. Compression is reversible by
the user via slash commands.
