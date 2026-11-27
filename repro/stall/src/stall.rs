//! A raw TCP proxy (TLS passes through untouched) between the real webzjs_wallet::Wallet and
//! the lightwalletd that forwards normally, then stops forwarding server->client in the middle of the
//! block-range stream while keeping both sockets open and sending nothing.
use std::{io::Read, sync::{atomic::{AtomicBool, AtomicU64, Ordering::SeqCst}, Arc}, time::{Duration, Instant}};
use tokio::{io::{AsyncReadExt, AsyncWriteExt}, net::{TcpListener, TcpStream}};
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

const STALL_AFTER_BYTES: u64 = 3_000_000;

#[tokio::main]
async fn main() {
    let url = server_url();
    let host = url.trim_start_matches("https://").trim_end_matches('/').to_string();
    let (domain, upstream) = match host.rsplit_once(':') { Some((d, _)) => (d.to_string(), host.clone()), None => (host.clone(), format!("{host}:443")) };
    let wait = Duration::from_secs(std::env::args().nth(1).map(|s| s.parse().unwrap()).unwrap_or(120));
    let s2c = Arc::new(AtomicU64::new(0));
    let stalled = Arc::new(AtomicBool::new(false));
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    {
        let (s2c, stalled, upstream) = (s2c.clone(), stalled.clone(), upstream.clone());
        tokio::spawn(async move {
            loop {
                let (mut c, _) = l.accept().await.unwrap();
                let mut u = TcpStream::connect(upstream.as_str()).await.unwrap();
                let (s2c, stalled) = (s2c.clone(), stalled.clone());
                tokio::spawn(async move {
                    let (mut cr, mut cw) = c.split();
                    let (mut ur, mut uw) = u.split();
                    let up = async { let _ = tokio::io::copy(&mut cr, &mut uw).await; };
                    let down = async {
                        let mut buf = vec![0u8; 16384];
                        loop {
                            if stalled.load(SeqCst) { std::future::pending::<()>().await; }
                            let n = match ur.read(&mut buf).await { Ok(0) | Err(_) => return, Ok(n) => n };
                            let total = s2c.fetch_add(n as u64, SeqCst) + n as u64;
                            if total > STALL_AFTER_BYTES && !stalled.swap(true, SeqCst) {
                                continue; // drop this chunk too: from here on nothing reaches the client
                            }
                            if cw.write_all(&buf[..n]).await.is_err() { return; }
                        }
                    };
                    tokio::join!(up, down);
                });
            }
        });
    }
    let ch = Channel::from_shared(format!("https://127.0.0.1:{port}")).unwrap()
        .tls_config(ClientTlsConfig::new().with_webpki_roots().domain_name(domain.as_str())).unwrap()
        .connect().await.expect("connect via proxy");
    // control: direct connection (not through the proxy), what WebWallet::get_latest_block's clone would do on a healthy link
    let direct = Channel::from_shared(url.clone()).unwrap()
        .tls_config(ClientTlsConfig::new().with_webpki_roots()).unwrap().connect().await.unwrap();
    let tip = CompactTxStreamerClient::new(direct.clone()).get_latest_block(ChainSpec {}).await.unwrap().into_inner().height as u32;
    let mut seed = [0u8; 32];
    std::fs::File::open("/dev/urandom").unwrap().read_exact(&mut seed).unwrap();
    let ufvk = UnifiedSpendingKey::from_seed(&Network::MainNetwork, &seed, zip32::AccountId::ZERO).unwrap().to_unified_full_viewing_key();
    let wallet = Arc::new(Wallet::new(MemoryWalletDb::new(Network::MainNetwork, PRUNING_DEPTH), ch, Network::MainNetwork, ConfirmationsPolicy::MIN).unwrap());
    wallet.import_ufvk("t", &ufvk, AccountPurpose::ViewOnly, Some(tip - 50_000), None).await.unwrap();
    println!("tip {tip}, birthday {}, proxy 127.0.0.1:{port} -> {upstream}, stall after {STALL_AFTER_BYTES} server->client bytes, bounded wait {wait:?}", tip - 50_000);
    let t0 = Instant::now();
    let ms = move || t0.elapsed().as_millis();
    let w = wallet.clone();
    let sync = tokio::spawn(async move { let r = w.sync().await; (ms(), r.map_err(|e| e.to_string())) });
    while !stalled.load(SeqCst) && !sync.is_finished() { tokio::time::sleep(Duration::from_millis(50)).await; }
    println!("t={} ms  proxy stalled (s2c bytes so far {}); sync finished? {}", ms(), s2c.load(SeqCst), sync.is_finished());
    tokio::time::sleep(Duration::from_secs(2)).await;
    let w = wallet.clone();
    let issued = ms();
    let a = tokio::spawn(async move { let r = w.get_wallet_summary().await.map(|s| s.is_some()); (ms(), format!("{r:?}")) });
    let mut dc = CompactTxStreamerClient::new(direct);
    let b = dc.get_latest_block(ChainSpec {}).await.map(|r| r.into_inner().height);
    println!("t={} ms  control get_latest_block on a separate healthy connection -> {b:?}", ms());
    let res_a = tokio::time::timeout(wait, a).await;
    println!("t={} ms  get_wallet_summary issued t={issued} ms: {}", ms(), match &res_a { Ok(r) => format!("RETURNED {:?}", r.as_ref().unwrap()), Err(_) => format!("DID NOT RETURN within {wait:?}") });
    println!("t={} ms  sync() finished? {}  (s2c bytes {})", ms(), sync.is_finished(), s2c.load(SeqCst));
    std::process::exit(0);
}
