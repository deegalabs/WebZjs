# scan_cached_blocks hangs with no rayon pool threads (zcash/librustzcash#3270)

A native program, no browser, no network, no wasm. It builds a one-block chain in memory, creates an account in a `zcash_client_memory` wallet, and calls `zcash_client_backend::data_api::chain::scan_cached_blocks`.

```
cargo run --release                                 # POOL=default: rayon's normal pool, the scan returns (exit 0)
POOL=current cargo run --release                    # a one-thread registry on the calling thread, as rayon-core
                                                    # falls back to on wasm32 without threads: the scan does not
                                                    # return (exit 2 after TIMEOUT_SECS, default 20)
```

With `Tasks::run_task` in `zcash_client_backend/src/scan.rs` calling `task.run()` inline instead of `rayon::spawn_fifo`, the `POOL=current` run returns too (checked earlier with a local patch of `zcash_client_backend` 0.25.0, not included here).
