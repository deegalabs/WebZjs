//! AccountBirthday::from_treestate at R-1 for R a multiple of 10,000, incl. R below NU6.3 (Ironwood).
use std::io::Read;
use tonic::transport::{Channel, ClientTlsConfig};
use webzjs_common::Network;
use zcash_client_backend::{
    data_api::{chain::ChainState, AccountBirthday, AccountPurpose, WalletRead, WalletWrite},
    proto::service::{compact_tx_streamer_client::CompactTxStreamerClient, BlockId, ChainSpec, TreeState},
    sync::run,
};
use zcash_client_memory::{MemBlockCache, MemoryWalletDb};
use zcash_keys::keys::UnifiedSpendingKey;
use zcash_protocol::consensus::{NetworkUpgrade, Parameters, BlockHeight};

/// Public mainnet lightwalletd. Override with LIGHTWALLETD_URL (default https://zec.rocks:443).
fn server_url() -> String {
    std::env::var("LIGHTWALLETD_URL").unwrap_or_else(|_| "https://zec.rocks:443".into())
}

fn describe(label: &str, ts: &TreeState, b: &AccountBirthday) {
    let cs: &ChainState = b.prior_chain_state();
    println!("{label}: TreeState.height={} sapling_tree.len={} orchard_tree.len={} ironwood_tree.len={}", ts.height, ts.sapling_tree.len(), ts.orchard_tree.len(), ts.ironwood_tree.len());
    println!("  AccountBirthday.height()={}  sapling_frontier.tree_size={}  orchard_frontier.tree_size={}  ironwood_frontier.tree_size={}  (ironwood frontier is_empty: {})",
        u32::from(b.height()), cs.final_sapling_tree().tree_size(), cs.final_orchard_tree().tree_size(), cs.final_ironwood_tree().tree_size(), cs.final_ironwood_tree().tree_size() == 0);
}

#[tokio::main]
async fn main() {
    println!("Sapling activation (zcash_protocol MainNetwork): {:?}", Network::MainNetwork.activation_height(NetworkUpgrade::Sapling));
    println!("NU6.3 activation (zcash_protocol): {:?}", Network::MainNetwork.activation_height(NetworkUpgrade::Nu6_3));
    let ch = Channel::from_shared(server_url()).unwrap().tls_config(ClientTlsConfig::new().with_webpki_roots()).unwrap().connect().await.unwrap();
    let mut c = CompactTxStreamerClient::new(ch);
    let tip = c.get_latest_block(ChainSpec {}).await.unwrap().into_inner().height;
    println!("tip {tip}");
    for r in [3_610_000u64, 3_420_000u64] {
        let ts = c.get_tree_state(BlockId { height: r - 1, ..Default::default() }).await.unwrap().into_inner();
        match AccountBirthday::from_treestate(ts.clone(), None) {
            Ok(b) => describe(&format!("R={r}: ACCEPTED"), &ts, &b),
            Err(e) => println!("R={r}: from_treestate REJECTED: {e:?}"),
        }
    }
    // full check for the boundary case: import at R=3,420,000 and sync to tip (crosses NU6.3 at 3,428,143)
    let r = 3_420_000u64;
    let ts = c.get_tree_state(BlockId { height: r - 1, ..Default::default() }).await.unwrap().into_inner();
    let b = AccountBirthday::from_treestate(ts, None).unwrap();
    let mut seed = [0u8; 32];
    std::fs::File::open("/dev/urandom").unwrap().read_exact(&mut seed).unwrap();
    let ufvk = UnifiedSpendingKey::from_seed(&Network::MainNetwork, &seed, zip32::AccountId::ZERO).unwrap().to_unified_full_viewing_key();
    let mut db = MemoryWalletDb::new(Network::MainNetwork, 100);
    db.import_account_ufvk("t", &ufvk, &b, AccountPurpose::ViewOnly, None).expect("import at R below NU6.3");
    println!("imported at R={r} (below NU6.3 3,428,143); syncing to tip ...");
    let cache = MemBlockCache::new();
    let mut client = c.clone();
    let t0 = std::time::Instant::now();
    let res = run(&mut client, &Network::MainNetwork, &cache, &mut db, 10_000).await;
    println!("sync::run -> {:?} after {:?}", res.as_ref().map(|_| "Ok").map_err(|e| e.to_string()), t0.elapsed());
    println!("chain_height {:?}  fully_scanned {:?}", db.chain_height().unwrap(), db.block_fully_scanned().unwrap().map(|m| u32::from(m.block_height())));
    println!("suggest_scan_ranges left: {}", db.suggest_scan_ranges().unwrap().len());
    let _ = BlockHeight::from(0u32);
}
