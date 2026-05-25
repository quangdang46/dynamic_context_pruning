//! 05_with_memory — implement [`MemoryRetriever`] and inject it.
//!
//! `MemoryRetriever` is the pluggable hook for cross-session memory
//! lookup (PLAN.md §3.4). DCP itself never persists across sessions —
//! the host owns whatever vector store / RAG pipeline it likes, and the
//! library just consults it. This example wires a tiny in-memory
//! keyword store so the call shape is visible end-to-end.

use std::sync::Arc;

use dynamic_context_pruning::{
    Config, ContextPruner, MemoryRetriever, Message, RetrievalError, RetrievedMemory,
};

/// Toy keyword retriever — scores each memory by the count of tokens
/// from `query` that appear in `content`.
///
/// A real host would back this with a vector store (qdrant, lance,
/// in-process FAISS, …). The point of the example is to show that the
/// trait surface is *just* `retrieve(query, k)`.
#[derive(Debug, Default)]
struct KeywordMemory {
    notes: Vec<RetrievedMemory>,
}

impl KeywordMemory {
    fn new() -> Self {
        Self {
            notes: vec![
                RetrievedMemory {
                    content: "The codebase uses serde with derive features.".into(),
                    score: 0.0,
                    source: Some("memory://serde".into()),
                },
                RetrievedMemory {
                    content: "Cargo.toml is protected from pruning.".into(),
                    score: 0.0,
                    source: Some("memory://config".into()),
                },
                RetrievedMemory {
                    content: "Default tokenizer is char/4.".into(),
                    score: 0.0,
                    source: Some("memory://tokenizer".into()),
                },
            ],
        }
    }
}

impl MemoryRetriever for KeywordMemory {
    fn retrieve(&self, query: &str, k: usize) -> Result<Vec<RetrievedMemory>, RetrievalError> {
        if query.is_empty() {
            return Err(RetrievalError::InvalidQuery("empty query".into()));
        }
        let needles: Vec<&str> = query.split_whitespace().collect();
        let mut scored: Vec<RetrievedMemory> = self
            .notes
            .iter()
            .map(|m| {
                let hits = needles
                    .iter()
                    .filter(|n| m.content.to_lowercase().contains(&n.to_lowercase()))
                    .count();
                RetrievedMemory {
                    content: m.content.clone(),
                    score: hits as f32 / needles.len().max(1) as f32,
                    source: m.source.clone(),
                }
            })
            .filter(|m| m.score > 0.0)
            .collect();
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        scored.truncate(k);
        Ok(scored)
    }
}

fn main() -> anyhow::Result<()> {
    // The host owns the retriever. DCP just borrows it through the
    // `Arc<dyn MemoryRetriever>` slot in the builder.
    let retriever: Arc<dyn MemoryRetriever> = Arc::new(KeywordMemory::new());

    // Direct access first — verifies the implementation in isolation.
    let hits = retriever.retrieve("serde derive", 2)?;
    println!("direct retrieval -> {} hits", hits.len());
    for h in &hits {
        println!(
            "  score={:.2} source={:?} :: {}",
            h.score, h.source, h.content
        );
    }

    // Wire it through the pruner.
    let mut pruner = ContextPruner::builder()
        .config(Config::default())
        .memory(Arc::clone(&retriever))
        .build()?;

    // The pipeline stores the retriever for future LLM-driven
    // enrichment hooks (PLAN.md §3.4 / §6.5). For now we round-trip a
    // small message stream to confirm the wiring compiles and that
    // memory introspection is reachable through the public API.
    let messages = vec![
        Message::user_text("u1", 0, "Remind me about serde derive."),
        Message::assistant_text("a1", 0, "Sure — looking up notes."),
    ];
    let pruned = pruner.transform_messages(messages)?;
    println!("pruned: {} messages", pruned.len());

    if let Some(mem) = pruner.memory() {
        let recalled = mem.retrieve("serde", 1)?;
        println!("via pruner.memory(): {} hit(s)", recalled.len());
    }
    Ok(())
}
