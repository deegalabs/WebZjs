//! Logs every request the real webzjs_wallet::Wallet sends to the server: import, first sync, second sync.
use std::{future::Future, io::Read, pin::Pin, sync::{Arc, Mutex}, task::{Context, Poll}};
use bytes::Buf;
use http_body_util::{BodyExt, Full};
use prost::Message;
use tonic::{body::Body as TonicBody, transport::{Channel, ClientTlsConfig}};
use webzjs_common::Network;
use webzjs_wallet::{Wallet, PRUNING_DEPTH};
use zcash_client_backend::{
    data_api::{wallet::ConfirmationsPolicy, AccountPurpose, WalletRead},
    proto::service::{compact_tx_streamer_client::CompactTxStreamerClient, BlockId, BlockRange, ChainSpec, GetAddressUtxosArg},
};
use zcash_client_memory::MemoryWalletDb;
use zcash_keys::keys::{UnifiedFullViewingKey, UnifiedSpendingKey};

/// Public mainnet lightwalletd. Override with LIGHTWALLETD_URL (default https://zec.rocks:443).
fn server_url() -> String {
    std::env::var("LIGHTWALLETD_URL").unwrap_or_else(|_| "https://zec.rocks:443".into())
}

type Log = Arc<Mutex<Vec<String>>>;
#[derive(Clone)]
struct Logging { inner: Channel, log: Log }

impl tower_service::Service<http::Request<TonicBody>> for Logging {
    type Response = <Channel as tower_service::Service<http::Request<TonicBody>>>::Response;
    type Error = <Channel as tower_service::Service<http::Request<TonicBody>>>::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> { Poll::Ready(Ok(())) }
    fn call(&mut self, req: http::Request<TonicBody>) -> Self::Future {
        let mut inner = self.inner.clone();
        let log = self.log.clone();
        Box::pin(async move {
            let (parts, body) = req.into_parts();
            let mut b = body.collect().await.expect("collect").to_bytes();
            let method = parts.uri.path().rsplit('/').next().unwrap().to_string();
            let mut p = b.clone();
            if p.len() >= 5 { p.advance(5); }
            let detail = match method.as_str() {
                "GetTreeState" => format!("{:?}", BlockId::decode(p).map(|x| x.height)),
                "GetBlockRange" => format!("{:?}", BlockRange::decode(p).map(|x| (x.start.map(|s| s.height), x.end.map(|s| s.height)))),
                "GetAddressUtxosStream" | "GetAddressUtxos" => format!("{:?}", GetAddressUtxosArg::decode(p).map(|x| (x.addresses.len(), x.start_height, x.max_entries))),
                "GetLatestBlock" => "ChainSpec{}".into(),
                _ => format!("{} bytes", p.len()),
            };
            log.lock().unwrap().push(format!("{method} {detail}"));
            let req = http::Request::from_parts(parts, TonicBody::new(Full::new(std::mem::take(&mut b))));
            std::future::poll_fn(|cx| inner.poll_ready(cx)).await?;
            inner.call(req).await
        })
    }
}

fn dump(title: &str, log: &Log) {
    println!("  [{title}]");
    for l in log.lock().unwrap().drain(..) { println!("    {l}"); }
}

async fn birthday_of(w: &Wallet<MemoryWalletDb<Network>, Logging>) -> Vec<u32> {
    let bytes = w.db_to_bytes().await.unwrap();
    let db = MemoryWalletDb::decode_new(bytes.as_slice(), Network::MainNetwork, PRUNING_DEPTH).unwrap();
    let ids = db.get_account_ids().unwrap();
    ids.iter().map(|id| u32::from(db.get_account_birthday(*id).unwrap())).collect()
}

#[tokio::main]
async fn main() {
    let url = server_url();
    let channel = Channel::from_shared(url.clone()).unwrap().tls_config(ClientTlsConfig::new().with_webpki_roots()).unwrap().connect().await.unwrap();
    let mut lc = CompactTxStreamerClient::new(channel.clone());
    let tip = lc.get_latest_block(ChainSpec {}).await.unwrap().into_inner().height as u32;
    let b = tip - 12_000;
    println!("server {url}  tip {tip}  B = tip-12000 = {b}  (B-1 = {})", b - 1);
    let mut seed = [0u8; 32];
    std::fs::File::open("/dev/urandom").unwrap().read_exact(&mut seed).unwrap();
    let usk = UnifiedSpendingKey::from_seed(&Network::MainNetwork, &seed, zip32::AccountId::ZERO).unwrap();
    let ufvk = usk.to_unified_full_viewing_key();
    println!("full ufvk: transparent={} sapling={} orchard={}", ufvk.transparent().is_some(), ufvk.sapling().is_some(), ufvk.orchard().is_some());
    let orch_only = UnifiedFullViewingKey::from_orchard_fvk(ufvk.orchard().unwrap().clone()).unwrap();
    println!("orchard-only ufvk: transparent={} sapling={} orchard={}", orch_only.transparent().is_some(), orch_only.sapling().is_some(), orch_only.orchard().is_some());

    let mk = |log: &Log| {
        let ch = Logging { inner: channel.clone(), log: log.clone() };
        Wallet::new(MemoryWalletDb::new(Network::MainNetwork, PRUNING_DEPTH), ch, Network::MainNetwork, ConfirmationsPolicy::MIN).unwrap()
    };
    for (name, key) in [("UFVK WITH transparent receivers, birthday B", &ufvk), ("Orchard-only UFVK, birthday B", &orch_only)] {
        println!("\n=== {name}");
        let log: Log = Default::default();
        let w = mk(&log);
        w.import_ufvk("t", key, AccountPurpose::ViewOnly, Some(b), None).await.unwrap();
        dump("import_ufvk", &log);
        println!("  stored birthday: {:?}", birthday_of(&w).await);
        w.sync().await.unwrap();
        dump("first sync()", &log);
        w.sync().await.unwrap();
        dump("second sync() (caught up)", &log);
    }
}
