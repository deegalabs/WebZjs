# sync() inside a Web Worker at #34 head (1a9c3ac5)

A browser check in Chromium (Playwright). A page starts a module Worker; the Worker imports the wallet package, builds `WebWallet("main", proxy, 3, 10, undefined)`, reads the tip, creates a throwaway account at tip - 100 and calls `sync()`. It reads public mainnet chain data only, through a small local gRPC-web proxy to a public lightwalletd (default `https://zec.rocks:443`, override with `UPSTREAM`).

Every key is generated at run time (the seed phrase comes from the wasm module's `generate_seed_phrase()`), never printed, never funded, and dropped. No transaction is built or sent. `create_proposed_transactions()` is not called: it needs a `Proposal`, which needs a funded wallet.

## Files

- `site/index.html`, `site/worker.js`: the page and the Worker.
- `run.cjs`: serves `site/`, opens it in Chromium, prints the Worker log and the browser console. Arguments: variant (`threaded` or `single`), `1` to send COOP/COEP headers or `0` not to, and the sync wait in seconds. Environment: `CHROMIUM_PATH` (a Chromium or Chrome binary), `SITE_DIR`, `PROXY_URL` (default `http://localhost:1234`).
- `grpcweb-proxy.cjs`: gRPC-web to gRPC proxy. Environment: `UPSTREAM`, `PORT` (default 1234).

## Build the two flavours (from the repo root, at this commit)

```
# threaded
just build-wallet
mkdir -p repro/worker-assert/site/pkg-threaded
cp -r packages/webzjs-wallet/. repro/worker-assert/site/pkg-threaded/

# no-threads
just build-wallet-single
mkdir -p repro/worker-assert/site/pkg-single
cp -r packages/webzjs-wallet-single/. repro/worker-assert/site/pkg-single/
```

Measured wasm sha256 (both builds, from a clean checkout of this commit): threaded `3033498646cb975d26861b9006c6f22af8d73effc2a2e844a63712a96a98d11a`, no-threads `02d268d475d569120e0a42e2e5b9bdfa11b56498a4d629b52bacd5181c562af8`.

## Run

```
cd repro/worker-assert
npm ci
node grpcweb-proxy.cjs &                       # start it right before run.cjs, see the note below
export CHROMIUM_PATH=/path/to/chromium         # or install one for playwright-core
node run.cjs threaded 1 60                     # threaded build, page cross-origin isolated
node run.cjs single 0 120                      # no-threads build, no COOP/COEP
node -e "require('playwright-core').chromium.launch({headless:true,executablePath:process.env.CHROMIUM_PATH}).then(async b=>{console.log(b.version());await b.close()})"   # prints the Chromium version
```

The proxy opens one HTTP/2 connection to the upstream at start and does not reconnect. In two runs where it had been idle for some seconds to tens of seconds before the first request, the Worker got `malformed response` from `get_latest_block` and the proxy logged `The session has been destroyed`; started immediately before `run.cjs`, it worked. Start it just before each run.

## Results measured on 2026-11-27 (headless Chromium 151.0.7922.34, playwright-core 1.65.1)

Raw output of each run, saved with the commit and the sha256 of the wasm.

Threaded, in a Worker (`crossOriginIsolated: true`):

```
variant=threaded COOP=same-origin COEP=require-corp
page crossOriginIsolated: true
worker: {"stage":"start","variant":"threaded","workerIsolated":true,"sab":"function"}
worker: {"stage":"wasm-initialised"}
worker: {"stage":"account-created","tip":3617785}
worker: {"stage":"worker-onerror","msg":"Uncaught RuntimeError: unreachable"}
--- browser console (last 25) ---
error: Failed to load resource: the server responded with a status of 404 (Not Found)
info: INFO webzjs_wallet::bindgen::wallet: Create account called
info: INFO webzjs_wallet::wallet: Key successfully decoded. Importing into wallet
info: INFO webzjs_wallet::wallet: Importing account
error: panicked at crates/webzjs-wallet/src/bindgen/wallet.rs:294:9:
assertion failed: !thread::is_web_worker_thread()

Stack:

Error
    at imports.wbg.__wbg_new_8a6f238a6ece86ea (http://localhost:8091/pkg-threaded/webzjs_wallet.js:2242:21)
    at http://localhost:8091/pkg-threaded/webzjs_wallet_bg.wasm:wasm-function[5858]:0x6d691d
    at http://localhost:8091/pkg-threaded/webzjs_wallet_bg.wasm:wasm-fun
pageerror: RuntimeError: unreachable
```

`sync()` panics at `bindgen/wallet.rs:294:9` (the assert is the first line of `sync`) and the Worker gets `RuntimeError: unreachable` through `onerror`. The `try/catch` around `await wallet.sync()` receives no rejection, and no `sync-threw` stage is posted.

No-threads, in a Worker (`crossOriginIsolated: false`, `SharedArrayBuffer` undefined):

```
variant=single COOP=- COEP=-
page crossOriginIsolated: false
worker: {"stage":"start","variant":"single","workerIsolated":false,"sab":"undefined"}
worker: {"stage":"wasm-initialised"}
worker: {"stage":"account-created","tip":3617786}
--- browser console (last 25) ---
error: Failed to load resource: the server responded with a status of 404 (Not Found)
info: INFO webzjs_wallet::bindgen::wallet: Create account called
info: INFO webzjs_wallet::wallet: Key successfully decoded. Importing into wallet
info: INFO webzjs_wallet::wallet: Importing account
info: INFO zcash_client_backend::sync: Sapling tree has 1129 subtrees
info: INFO zcash_client_backend::sync: Orchard tree has 770 subtrees
info: INFO zcash_client_backend::sync: Ironwood tree has 21 subtrees
info: INFO zcash_client_backend::sync: Latest block height is 3617786
info: INFO zcash_client_backend::sync: Refreshing UTXOs for AccountId(1) from height 3617686
debug: DEBUG zcash_client_backend::sync: Suggested ranges: [ScanRange { block_range: BlockHeight(3617686)..BlockHeight(3617787), priority: ChainTip }]
info: INFO zcash_client_backend::sync: Fetching ChainTip(3617686..3617787)
info: INFO zcash_client_backend::sync: Scanning ChainTip(3617686..3617787)
```

No trap. `sync()` starts scanning and does not return: the Worker's own 120 s `setTimeout` guard never fired and no later stage was posted before the runner's 270 s deadline (the 120 s Worker timer plus 150 s), so the Worker thread itself was blocked. The PR description ties this hang to zcash/librustzcash#3270 (a blocking wait inside `scan_cached_blocks` on a thread pool that does not exist); this run shows the Worker blocked, not the cause.

Limits: the assert at `bindgen/wallet.rs:437` (`create_proposed_transactions`) was read, not run.
