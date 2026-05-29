//! Generate `dcp.schema.json` at the workspace root.
//!
//! Run with: `cargo run -p dcp-config --bin generate-schema`
//!
//! This binary is the **only** way the schema file is written.
//! It is intentionally NOT invoked during `cargo test` — tests
//! validate the schema *value* in memory, while this binary
//! handles the one-time disk write.

use std::env;
use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Allow overriding the output path via env var (useful for CI).
    let output_path = env::var("DCP_SCHEMA_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            // Default: workspace root (parent of crates/dcp-config/).
            let manifest_dir =
                env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set by cargo");
            PathBuf::from(&manifest_dir)
                .parent()
                .expect("crate must have a parent")
                .parent()
                .expect("workspace must have a root")
                .join("dcp.schema.json")
        });

    let schema = dcp_config::json_schema();
    let formatted =
        serde_json::to_string_pretty(&schema).expect("schemars must produce valid JSON");

    fs::write(&output_path, &formatted)?;
    println!("Schema written to {}", output_path.display());
    println!(
        "  {} bytes, {} top-level keys",
        formatted.len(),
        schema.as_object().map(|o| o.len()).unwrap_or(0)
    );

    Ok(())
}
