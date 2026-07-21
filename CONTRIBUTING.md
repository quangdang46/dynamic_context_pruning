# Contributing to DCP

Thank you for your interest in contributing to Dynamic Context Pruning (DCP)!

## License and Contributions

This project uses the **GNU Affero General Public License v3.0 (AGPL-3.0)**.

### Contribution Agreement

By submitting a Pull Request to this project, you agree that:

1.  Your contributions are licensed under the **AGPL-3.0**.
2.  You grant the project maintainer(s) a non-exclusive, perpetual, irrevocable, worldwide, royalty-free, transferable license to use, modify, and re-license your contributions under any terms they choose, including commercial or proprietary licenses.

This arrangement ensures the project remains Open Source while providing a path for commercial sustainability.

## Getting Started

### Prerequisites

- **Rust 1.85+** (edition 2024). Install via [rustup](https://rustup.rs/).
- **cargo** (included with rustup).

### Build

```bash
cargo build --workspace
```

### Test

```bash
cargo test --workspace
```

Run tests for a single crate:

```bash
cargo test -p dcp-messages
cargo test -p dcp-config
cargo test -p dcp-state
```

### Lint

```bash
cargo clippy --workspace -- -D warnings
```

### Format

```bash
cargo fmt --workspace
```

### All Checks

Run build, test, lint, and format in one pass:

```bash
cargo fmt --workspace -- --check \
  && cargo clippy --workspace -- -D warnings \
  && cargo test --workspace \
  && cargo build --workspace
```

## Pre-commit Hooks

This project uses a pre-commit hook to enforce quality gates **before every commit**.

### Install

```bash
./scripts/install-hooks.sh
```

This symlinks `scripts/pre-commit.sh` into `.git/hooks/pre-commit`.

### What it runs (5 gates)

| Gate | Command | Blocks on |
|------|---------|-----------|
| Formatting | `cargo fmt --all -- --check` | Any formatting violation |
| Linting | `cargo clippy --all-targets --all-features -- -D warnings` | Any warning |
| Tests | `cargo test --all-features` | Any test failure |
| Build | `cargo build --all-features` | Any compilation error |
| Audit | `cargo audit` | Known crate vulnerabilities |

All 5 must pass before `git commit` completes.

### Bypass (emergency only)

```bash
git commit --no-verify -m "emergency fix"
```

### Per-bead code review

To verify a specific bead's implementation before closing it:

```bash
# Review a specific bead (e.g. get-message command)
./scripts/review-bead.sh 0ed

# Review all beads (full workspace)
./scripts/review-bead.sh --all

# List all beads and their associated files
./scripts/review-bead.sh --list
```

The `review-bead.sh` script maps bead IDs to their implementation files and runs the full quality gate on the relevant crate(s).

### Every bead needs a clean code review

Before closing a bead (updating its `status` to `"closed"` in `.beads/issues.jsonl`):

1. Run `./scripts/review-bead.sh <BEAD_ID>` for the bead's implementation files
2. All 4 gates must pass
3. Add the verification result to the bead's `close_reason` field

Example close reason:
```json
"close_reason": "Verified: cargo fmt OK, clippy OK, all tests pass, build OK"
```

## Workspace Structure

This is a Cargo workspace with 21 crates. Key crates:

| Crate | Purpose |
|-------|---------|
| `dcp-config` | Configuration schema, JSONC parser, cascade resolution |
| `dcp-state` | Session state management, message refs, compression blocks |
| `dcp-messages` | Message query, shape, sync, priority, injection |
| `dcp-permissions` | HTTP auth, host permissions, compress permission resolution |
| `dcp-prompts` | Prompt store with 3-tier override cascade, extensions |
| `dcp-notification` | Notification formatting and delivery |
| `dcp-core` | Core transform pipeline |

See `README.md` for the full architecture diagram.

## Pull Request Process

1. Fork the repository.
2. Create a feature branch (`git checkout -b feature/your-change`).
3. Implement your changes. **Add tests for all new behavior.**
4. Run all checks (see "All Checks" above).
5. Commit with a descriptive message. Follow [Conventional Commits](https://www.conventionalcommits.org/) where possible.
6. Submit a Pull Request against `main`.

## Coding Standards

- **No `as any` or type suppression.** Rust's type system is your friend.
- **No `#[allow(clippy::...)]`** without a documented justification in a comment.
- **No empty `catch` blocks.** Errors must be handled or propagated.
- **Tests first.** Write failing tests before implementing new behavior.
- **Follow existing patterns.** Read neighboring files for style guidance.
- **No AI slop.** Code should read like it was written by a senior engineer — clear names, no unnecessary abstractions, no speculative generality.

## Configuration Schema

The JSON schema for `opencode.json` lives at `opencode-dcp-plugin/dcp.schema.json`, generated from Rust types via `schemars`. After changing any config type, update the schema file manually to match.

## Beads (Issue Tracking)

This project uses `.beads/issues.jsonl` for local-first issue tracking. Each bead is a JSON line with `id`, `title`, `status`, and `priority`. Close beads by updating `status` to `"closed"`.

We look forward to your contributions!
