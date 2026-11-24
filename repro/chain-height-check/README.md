# chain_height() after a sync that stops partway (red-dev-inc/WebZjs#29)

A native program, no browser, no network. It drives `zcash_client_backend` 0.25.0 `sync::run` against a fake lightwalletd (an in-process tower service) with the wallet store WebZjs pins (`zcash_client_memory` b4c8f0d), birthday 3,500,000, tip 3,524,999, empty blocks, and prints `chain_height`, `block_fully_scanned` and `block_max_scanned` after each sync.

```
cargo run --release --locked -- B   # the first GetBlockRange ends after 500 of 10,000 blocks with an OK status
cargo run --release --locked -- E   # the second GetBlockRange sends 300 blocks, then grpc-status 11
```

Observed (Linux x86_64, release build):

- B: `sync #1: run() -> Ok(())`, then `chain_height Some(BlockHeight(3524999))  block_fully_scanned Some(BlockHeight(3500499))`.
- E: `sync #1: run() -> Err(... out of range)`, then `chain_height Some(BlockHeight(3524999))  block_fully_scanned Some(BlockHeight(3509999))`.

In both, a check against `chain_height()` sees the wallet at the tip, while the fully scanned height is 24,500 (B) and 15,000 (E) blocks below it. A second `sync` fills the gap in both.
