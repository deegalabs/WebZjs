# Birthday, lock and stall checks at #27 head (9ad0a589)

Native programs, no browser. `lock`, `leak`, `none` and `stall` call the real `webzjs_wallet::Wallet` (`import_ufvk`, `create_account`, `sync`, `get_wallet_summary`, `db_to_bytes`), the Rust methods the wasm bindings wrap, over a native tonic channel. `birthday` does not use `Wallet`: it calls `AccountBirthday::from_treestate`, an in-memory store and `zcash_client_backend::sync::run` directly. They read public mainnet chain data only, from a public lightwalletd (default `https://zec.rocks:443`, override with `LIGHTWALLETD_URL`).

Every key is generated at run time from `/dev/urandom`, never printed, never funded, and dropped. Accounts are view-only or throwaway and hold no notes. Nothing is sent to the network except read requests.

This is a standalone crate (its own `[workspace]`, committed `Cargo.lock`, path dependencies on `../../crates/...`). Build from this folder; the repo's `rust-toolchain.toml` (nightly-2025-06-15) applies.

```
cargo build --release --locked
```

## What each binary shows

- `lock`: with a 50,000-block sync running, `get_wallet_summary` returns only after `sync` returns, while a tip read, made with a new client on a clone of the same connection, answers in 18 to 58 ms.
- `leak`: the requests the wallet sends (height of `GetTreeState`, range of `GetBlockRange`, `start_height` of `GetAddressUtxosStream`) on import, first sync and second sync, for a UFVK with transparent receivers and for an Orchard-only UFVK.
- `none`: `import_ufvk` and `create_account` with birthday `None` store tip - 100, read back from the account.
- `birthday`: `AccountBirthday::from_treestate` at a multiple of 10,000 (3,610,000) and below NU6.3 (3,420,000), then a sync from 3,420,000 to the tip across the NU6.3 activation.
- `stall`: a TCP proxy stops forwarding server-to-client bytes after a fixed 3,000,000 bytes from the server and keeps the sockets open; `sync()` and `get_wallet_summary` do not return within 120 s while a separate healthy connection answers (22 ms or less in the run below). The proxy counts bytes, not requests, and the request in flight when it stops is not logged.

## Commands and results measured on 2026-11-27 (Linux x86_64, release build)

Every block below is the raw output of one run, saved with the commit and the sha256 of the binary. The tip moves between runs, so the block numbers differ from run to run.

```
./target/release/lock 50000
```

Three runs, birthday = tip - 50,000. The summary call and the tip read are issued while the sync is still running. Latency is resolved minus issued as the program prints it, an upper bound that includes spawning the task.

| run | tip read: issued, resolved | get_wallet_summary: issued, resolved | sync returned |
|---|---|---|---|
| 1 | 3001 ms, 3019 ms (at most 18 ms) | 3001 ms, 38737 ms (35,736 ms wait) | 38,734 ms |
| 2 | 8939 ms, 8995 ms (at most 56 ms) | 8939 ms, 39376 ms (30,437 ms wait) | 39,374 ms |
| 3 | 7494 ms, 7552 ms (at most 58 ms) | 7494 ms, 37575 ms (30,081 ms wait) | 37,573 ms |

The summary resolved 3, 2 and 2 ms after `sync()` returned and carried `Some((tip, tip))`. In runs 2 and 3 the calls were issued at 8.9 s and 7.5 s after the start, not at 3 s; both were issued after the sync started and before it ended.

```
./target/release/leak
```

Output of the run (tip 3,617,725, B = tip - 12,000 = 3,605,725):

```
server https://zec.rocks:443  tip 3617725  B = tip-12000 = 3605725  (B-1 = 3605724)
full ufvk: transparent=true sapling=true orchard=true
orchard-only ufvk: transparent=false sapling=false orchard=true

=== UFVK WITH transparent receivers, birthday B
  [import_ufvk]
    GetTreeState Ok(3605724)
  stored birthday: [3605725]
  [first sync()]
    GetSubtreeRoots 0 bytes
    GetSubtreeRoots 2 bytes
    GetSubtreeRoots 2 bytes
    GetLatestBlock ChainSpec{}
    GetAddressUtxosStream Ok((1, 3605725, 0))
    GetBlockRange Ok((Some(3605725), Some(3615724)))
    GetTreeState Ok(3605724)
    GetBlockRange Ok((Some(3615725), Some(3617725)))
    GetTreeState Ok(3615724)
  [second sync() (caught up)]
    GetSubtreeRoots 0 bytes
    GetSubtreeRoots 2 bytes
    GetSubtreeRoots 2 bytes
    GetLatestBlock ChainSpec{}
    GetAddressUtxosStream Ok((1, 3605725, 0))

=== Orchard-only UFVK, birthday B
  [import_ufvk]
    GetTreeState Ok(3605724)
  stored birthday: [3605725]
  [first sync()]
    GetSubtreeRoots 0 bytes
    GetSubtreeRoots 2 bytes
    GetSubtreeRoots 2 bytes
    GetLatestBlock ChainSpec{}
    GetBlockRange Ok((Some(3605725), Some(3615724)))
    GetTreeState Ok(3605724)
    GetBlockRange Ok((Some(3615725), Some(3617726)))
    GetTreeState Ok(3615724)
  [second sync() (caught up)]
    GetSubtreeRoots 0 bytes
    GetSubtreeRoots 2 bytes
    GetSubtreeRoots 2 bytes
    GetLatestBlock ChainSpec{}
```

Reading it:

- import (`import_ufvk`, birthday B): one request, `GetTreeState` at B-1 = 3,605,724, for both key types.
- first sync, UFVK with transparent receivers: `GetAddressUtxosStream` with 1 address and `start_height` 3,605,725 (= B); `GetBlockRange` 3,605,725 to 3,615,724; `GetTreeState` at 3,605,724; `GetBlockRange` 3,615,725 to 3,617,725; `GetTreeState` at 3,615,724.
- second sync (caught up), same key: `GetAddressUtxosStream` again with `start_height` 3,605,725.
- first sync, Orchard-only UFVK: no `GetAddressUtxosStream` at all; `GetBlockRange` and `GetTreeState` at 3,605,724 are still sent (the tip had moved to 3,617,726 by then). Its second sync sends no request beyond `GetSubtreeRoots` and `GetLatestBlock`.
- the stored birthday is 3,605,725 in both cases.

```
./target/release/none
```

Output of the run (the stored birthday equals the tip read just after the import minus 100, exactly, for both paths):

```
server https://zec.rocks:443

=== UFVK path, birthday None
  [import_ufvk(None)]
    GetLatestBlock ChainSpec{}
    GetTreeState Ok(3617625)
  stored birthday: [3617626]   tip read just after: 3617726   tip-100 = 3617626

=== seed path (create_account), birthday None
  [create_account(None)]
    GetLatestBlock ChainSpec{}
    GetTreeState Ok(3617625)
  stored birthday: [3617626]   tip read just after: 3617726   tip-100 = 3617626
```

```
./target/release/birthday      # about 3.5 minutes: it syncs 197,734 blocks
```

Measured at chain tip 3,617,734:

```
Sapling activation (zcash_protocol MainNetwork): Some(BlockHeight(419200))
NU6.3 activation (zcash_protocol): Some(BlockHeight(3428143))
tip 3617734
R=3610000: ACCEPTED: TreeState.height=3609999 sapling_tree.len=1220 orchard_tree.len=964 ironwood_tree.len=1028
  AccountBirthday.height()=3610000  sapling_frontier.tree_size=74004476  orchard_frontier.tree_size=50485048  ironwood_frontier.tree_size=1424844  (ironwood frontier is_empty: false)
R=3420000: ACCEPTED: TreeState.height=3419999 sapling_tree.len=900 orchard_tree.len=1092 ironwood_tree.len=0
  AccountBirthday.height()=3420000  sapling_frontier.tree_size=73935208  orchard_frontier.tree_size=50208195  ironwood_frontier.tree_size=0  (ironwood frontier is_empty: true)
imported at R=3420000 (below NU6.3 3,428,143); syncing to tip ...
sync::run -> Ok("Ok") after 215.131601673s
chain_height Some(BlockHeight(3617734))  fully_scanned Some(3617734)
suggest_scan_ranges left: 0
```

The account is view-only and holds no notes, so this shows the tree sizes line up across the activation, not note handling.

```
./target/release/stall 120
```

Birthday tip - 50,000; the proxy stops after 3,000,000 server-to-client bytes (the chunk that crosses the threshold is dropped too, hence the count above 3,000,000).

```
tip 3617726, birthday 3567726, proxy 127.0.0.1:33555 -> zec.rocks:443, stall after 3000000 server->client bytes, bounded wait 120s
t=1555 ms  proxy stalled (s2c bytes so far 3006474); sync finished? false
t=3579 ms  control get_latest_block on a separate healthy connection -> Ok(3617727)
t=123580 ms  get_wallet_summary issued t=3557 ms: DID NOT RETURN within 120s
t=123580 ms  sync() finished? false  (s2c bytes 3006474)
```

The wait is bounded at 120 s, so this shows a hang for that long, not forever. The control call resolved 22 ms after the summary call was issued (3,557 ms to 3,579 ms), an upper bound that includes spawning the summary task. The transport is native tonic, not the browser fetch client.
