// Runs inside a dedicated Worker. Throwaway account, never funded, public mainnet reads only.
const q = new URL(import.meta.url).searchParams;
const variant = q.get("variant"), proxy = q.get("proxy"), syncWait = Number(q.get("syncWait") || 90) * 1000;
const post = (o) => postMessage(o);
const errs = (e) => String((e && e.stack) || e).slice(0, 400);
try {
  const wz = await import(`./pkg-${variant}/webzjs_wallet.js`);
  post({ stage: "start", variant, workerIsolated: self.crossOriginIsolated, sab: typeof SharedArrayBuffer });
  await wz.default();
  post({ stage: "wasm-initialised" });
  const wallet = new wz.WebWallet("main", proxy, 3, 10, undefined);
  const tip = Number(await wallet.get_latest_block());
  const seed = wz.generate_seed_phrase();
  await wallet.create_account("throwaway", seed, 0, tip - 100);
  post({ stage: "account-created", tip });
  const t0 = performance.now();
  try {
    const r = await Promise.race([
      wallet.sync().then(() => "returned"),
      new Promise((r) => setTimeout(() => r("NO RETURN within " + syncWait / 1000 + " s"), syncWait)),
    ]);
    post({ stage: "sync", result: r, ms: Math.round(performance.now() - t0) });
  } catch (e) { post({ stage: "sync-threw", error: errs(e), ms: Math.round(performance.now() - t0) }); }
  // a second call after a possible trap, to see whether the instance is still usable
  try {
    const t = await Promise.race([wallet.get_latest_block().then((x) => "ok " + x), new Promise((r) => setTimeout(() => r("NO RETURN in 15 s"), 15000))]);
    post({ stage: "after-sync get_latest_block", result: t });
  } catch (e) { post({ stage: "after-sync get_latest_block threw", error: errs(e) }); }
  post({ stage: "end", final: true });
} catch (e) { post({ stage: "setup-error", error: errs(e), final: true }); }
