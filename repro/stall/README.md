# A stall inside one batch at #18 head (53a47ed8)

A native program, no browser. It drives the real `webzjs_wallet::Wallet` at the head of PR #18, where `sync` is a loop of `sync_step` calls and the write lock is released between batches and held across one `run_step`. It reads public mainnet chain data only, from a public lightwalletd (default `https://zec.rocks:443`, override with `LIGHTWALLETD_URL`).

Every key is generated at run time from `/dev/urandom`, never printed, never funded, and dropped. The account is view-only and holds no notes.

## What it shows

A TCP proxy on 127.0.0.1 forwards bytes to the server (TLS stays end to end), then after a fixed 3,000,000 server-to-client bytes stops forwarding in that direction and keeps both sockets open. The proxy counts bytes, not requests, so which request the stall falls in (and which batch) is not logged. While a batch is being fetched and scanned the write lock is held, so the release between batches does not help. `sync()` does not return and `get_wallet_summary` does not return within 120 s, while a control `get_latest_block` on a separate healthy connection answers in 22 ms or less.

## Commands

Standalone crate (own `[workspace]`, committed `Cargo.lock`, path dependencies on `../../crates/...`). Run from this folder; the repo's `rust-toolchain.toml` applies.

```
cargo build --release --locked
./target/release/stall 120
```

The argument is the bounded wait in seconds (default 120).

## Result measured on 2026-11-27 (Linux x86_64, release build)

Raw output of the run, saved with the commit and the sha256 of the binary:

```
tip 3617745, birthday 3567745, proxy 127.0.0.1:33417 -> zec.rocks:443, stall after 3000000 server->client bytes, bounded wait 120s
t=2342 ms  proxy stalled (s2c bytes so far 3003290); sync finished? false
t=4364 ms  control get_latest_block on a separate healthy connection -> Ok(3617745)
t=124366 ms  get_wallet_summary issued t=4342 ms: DID NOT RETURN within 120s
t=124366 ms  sync() finished? false  (s2c bytes 3003290)
```

The control call resolved 22 ms after the summary call was issued (4,342 ms to 4,364 ms), an upper bound that includes spawning the summary task. The bound is 120 s, so this shows a hang for that long, not forever. The transport is native tonic, not the browser fetch client. The harness builds its channel in `stall.rs` without a timeout or keepalive; what the browser client configures was not run.
