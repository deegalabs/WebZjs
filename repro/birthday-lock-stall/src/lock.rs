//! Does get_wallet_summary wait for a running sync()? Uses the real webzjs_wallet::Wallet
//! (sync / import_ufvk / get_wallet_summary), over a native tonic Channel to a public lightwalletd.
use std::{io::Read, sync::Arc, time::Instant};
use tonic::transport::{Channel, ClientTlsConfig};
use webzjs_common::Network;
use webzjs_wallet::{Wallet, PRUNING_DEPTH};
use zcash_client_backend::{
    data_api::{wallet::ConfirmationsPolicy, AccountPurpose},
    proto::service::{compact_tx_streamer_client::CompactTxStreamerClient, ChainSpec},
};
use zcash_client_memory::MemoryWalletDb;
use zcash_keys::keys::UnifiedSpendingKey;

/// Public mainnet lightwalletd. Override with LIGHTWALLETD_URL (default https://zec.rocks:443).
fn server_url() -> String {
    std::env::var("LIGHTWALLETD_URL").unwrap_or_else(|_| "https://zec.rocks:443".into())
}

#[tokio::main]
async fn main() {
    let url = server_url();
    let back: u32 = std::env::args().nth(1).map(|s| s.parse().unwrap()).unwrap_or(50_000);
    let channel = Channel::from_shared(url.clone())
        .unwrap()
        .tls_config(ClientTlsConfig::new().with_webpki_roots())
        .unwrap()
        .connect()
        .await
        .expect("connect");
    // what WebWallet::get_latest_block does: a clone of the client, no wallet lock
    let mut lc = CompactTxStreamerClient::new(channel.clone());
    let tip = lc.get_latest_block(ChainSpec {}).await.unwrap().into_inner().height as u32;
    println!("server {url} tip {tip}, birthday {}", tip - back);

    let mut seed = [0u8; 32];
    std::fs::File::open("/dev/urandom").unwrap().read_exact(&mut seed).unwrap();
    let usk = UnifiedSpendingKey::from_seed(&Network::MainNetwork, &seed, zip32::AccountId::ZERO).unwrap();
    let ufvk = usk.to_unified_full_viewing_key(); // throwaway, never funded, seed dropped
    drop(seed);

    let db = MemoryWalletDb::new(Network::MainNetwork, PRUNING_DEPTH);
    let wallet = Arc::new(
        Wallet::new(db, channel.clone(), Network::MainNetwork, ConfirmationsPolicy::MIN).unwrap(),
    );
    wallet
        .import_ufvk("t", &ufvk, AccountPurpose::ViewOnly, Some(tip - back), None)
        .await
        .expect("import");

    let t0 = Instant::now();
    let ms = move || t0.elapsed().as_millis();
    let w = wallet.clone();
    let sync = tokio::spawn(async move {
        let r = w.sync().await;
        (ms(), r.map_err(|e| e.to_string()))
    });
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    println!("t={:>6} ms  sync started (3 s ago), issuing (a) and (b)", ms());
    let (wa, ca) = (wallet.clone(), CompactTxStreamerClient::new(channel.clone()));
    let a_issue = ms();
    let a = tokio::spawn(async move {
        let r = wa.get_wallet_summary().await.map(|s| s.map(|s| (s.chain_tip_height(), s.fully_scanned_height())));
        (ms(), format!("{r:?}"))
    });
    let b_issue = ms();
    let b = tokio::spawn(async move {
        let mut c = ca;
        let r = c.get_latest_block(ChainSpec {}).await.map(|r| r.into_inner().height);
        (ms(), format!("{r:?}"))
    });
    let (bt, bres) = b.await.unwrap();
    println!("(b) get_latest_block: issued t={b_issue} ms, resolved t={bt} ms (latency {} ms) -> {bres}", bt - b_issue);
    let (st, sres) = sync.await.unwrap();
    println!("sync: returned t={st} ms -> {sres:?}");
    let (at, ares) = a.await.unwrap();
    println!("(a) get_wallet_summary: issued t={a_issue} ms, resolved t={at} ms (latency {} ms; {} ms after sync returned) -> {ares}", at - a_issue, at as i128 - st as i128);
}
