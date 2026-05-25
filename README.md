# dynamic_context_pruning

Dynamic context pruning library for LLM coding agents.

> **Status**: Pre-alpha · scaffolding in progress · API not yet stable.

## What it does

Reduces token usage in LLM agent conversations by:

1. **Deterministic strategies** (per-turn, free): deduplicate identical tool calls, purge inputs of errored tools, remove stale file reads.
2. **LLM-driven compression**: a `compress` tool the model invokes to replace verbatim ranges with summaries.
3. **Cache-aware nudging**: avoids busting prompt caches on Anthropic / Bedrock / Gemini.

## Quick start

```rust
use dynamic_context_pruning::{ContextPruner, Config};

let mut pruner = ContextPruner::new(Config::load_default()?)?;
let pruned = pruner.transform_messages(messages)?;
```

## License

MIT — see [LICENSE](./LICENSE).

## Plan

See [PLAN.md](./PLAN.md) for the full design document.
