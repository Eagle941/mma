# AGENTS.md

## Scope

These instructions apply to the entire repository. If a more specific `AGENTS.md`
is added below a subdirectory, follow that file for work in its subtree.

## Repository overview

This repository is a Rust 2024 workspace for a market-making application. Keep
changes focused and preserve the separation between crates:

- `cli`: binary entry point and worker-thread wiring (`mma` binary).
- `configuration`: environment and credential loading.
- `exchange`: exchange-neutral trading types plus the Bybit integration.
- `strategy`: quote-generation logic and the strategy runner.
- `oms`: order lifecycle, risk checks, inventory, and execution accounting.
- `recorder`: execution markout recording.
- `integration-tests`: deterministic, offline end-to-end market simulation.

Prefer defining shared exchange-facing types and traits in `exchange` rather than
coupling other crates directly to Bybit. Keep orchestration in `cli` and business
logic in the relevant library crate.

## Toolchain and dependencies

- Use the Rust version pinned in `rust-toolchain.toml` (currently `1.96.0`).
- Keep `Cargo.toml`, `rust-toolchain.toml`, and `rustfmt.toml` version/edition
  comments in sync when changing the Rust version or edition.
- Reuse workspace dependencies via `dependency.workspace = true` when a crate is
  already listed under `[workspace.dependencies]`.
- Do not update unrelated dependencies or regenerate `Cargo.lock` unless the
  task requires it.
- Linux builds require `build-essential`, `pkg-config`, and `libssl-dev` (or
  equivalent packages).

## Build and validation

Start with the narrowest check that covers the change, then run broader checks
when practical.

```bash
# Targeted crate tests
cargo test -p <crate-name> --locked

# Offline integration test
cargo test -p integration-tests --test offline_market --locked

# CI-equivalent build and tests
cargo build --locked
cargo test --workspace --all-features --locked

# CI-equivalent lint
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings

# CI-equivalent formatting check (uses unstable rustfmt options)
rustup toolchain install nightly-2026-07-20 --profile minimal --component rustfmt
cargo +nightly-2026-07-20 fmt --all -- --check

# CI-equivalent documentation check
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps \
  --document-private-items --locked
```

Use `cargo +nightly-2026-07-20 fmt --all` to apply formatting. Do not replace the
pinned nightly formatter without also updating CI deliberately.

When dependencies change, run `cargo audit` if available. The CI audit currently
has documented temporary exceptions for `RUSTSEC-2025-0009` and
`RUSTSEC-2023-0065`, both inherited through `rust-bybit`; do not add further
exceptions without documenting the dependency and reason.

## Testing expectations

- Add or update unit tests beside the code under `#[cfg(test)]`.
- Use `rstest` for table-driven cases where it improves clarity.
- Put cross-crate behavior in `integration-tests`; keep those tests deterministic
  and offline by using the simulated exchange.
- Cover trading invariants and failure paths, including duplicate/stale events,
  order state transitions, inventory changes, precision, and invalid numeric
  input.
- Use approximate assertions for floating-point calculations where exact equality
  is not guaranteed.
- Do not make ordinary tests depend on credentials, wall-clock timing, or network
  access.

## Code conventions

- Follow existing Rust naming and module organization; let the pinned formatter
  enforce layout (`max_width = 100`).
- Keep Clippy clean with warnings denied. Do not suppress lints broadly just to
  make checks pass.
- Return descriptive errors for recoverable boundary failures. Add context with
  `anyhow::Context` where it makes failures actionable.
- Reserve `unwrap`, `expect`, and assertions for tests or invariants that have
  been validated explicitly. Do not introduce panics for malformed exchange or
  user-controlled data.
- Document public API semantics when behavior is not obvious, especially whether
  an operation means “dispatched,” “accepted,” or “executed.”
- Avoid unrelated refactors and preserve existing public APIs unless the task
  requires a breaking change.

## Trading and concurrency safety

This software can submit real orders. Treat behavior changes in `exchange`,
`strategy`, `oms`, configuration defaults, and thread wiring as safety-critical.

- Preserve order-side semantics, units, tick/quantity precision, monotonic order
  IDs, state transitions, execution deduplication, and stale-update handling.
- Validate external numeric values for finiteness, sign, range, and precision
  before they influence prices, quantities, inventory, or PnL.
- Preserve the distinction between command dispatch and exchange confirmation;
  exchange responses arrive asynchronously.
- Keep tests for any change to order submission, amendment, cancellation,
  repayment, inventory, or average-entry-price behavior.
- Respect the existing communication semantics: capacity-one `ArrayQueue`s carry
  only the latest market/inventory value, channels carry event streams, and the
  strategy currently runs at approximately 1 Hz.
- Do not replace synchronization primitives or change blocking/thread behavior
  without checking shutdown, disconnection, backpressure, ordering, and data-loss
  implications.
- Prefer deterministic offline simulation before any manual test against an
  exchange.

## Credentials and live-network rules

- Never read, print, edit, commit, or expose `.secrets`, API keys, API secrets, or
  signed request material. Credentials must remain redacted in `Debug` and logs.
- Do not weaken `.gitignore` protection for `.secrets` or generated `*.log` files.
- Do not run `cargo run`, `run.sh`, or `test_request.sh` unless the user explicitly
  requests a live-network action and confirms whether testnet or mainnet is
  intended. `cargo run` starts exchange-connected workers and may submit orders.
- Do not add live exchange calls to builds, unit tests, integration tests, or
  benchmarks.
- Default examples and test fixtures to fake credentials and testnet/offline
  behavior; never invent instructions that encourage committing real secrets.

## Change checklist

Before finishing:

1. Confirm the change is contained in the appropriate crate and does not create
   unnecessary cross-crate coupling.
2. Add or update tests for behavior changes, especially trading-state changes.
3. Run the narrowest relevant tests, formatting check, and Clippy; run the full
   workspace suite when practical.
4. Report exactly which commands were run and any checks that could not be run.
5. Call out behavior that could affect live trading, configuration, API
   compatibility, timing, ordering, or resource usage.
