//! Native repro: `scan_cached_blocks` waits forever when rayon has no pool threads.
//!
//! `POOL=default`  uses rayon's normal pool: the scan returns.
//! `POOL=current`  installs a one-thread registry that runs on the calling thread, which is what rayon-core
//!                 falls back to on wasm32 without threads: `spawn_fifo` jobs are queued but never run, and
//!                 the scan blocks reading the batch results.
//! Exit codes: 0 scan returned, 2 scan did not return within TIMEOUT_SECS (default 20).
use std::{sync::mpsc, time::Duration};

use zcash_client_backend::{
    data_api::{
        chain::{scan_cached_blocks, BlockSource, ChainState},
        AccountBirthday, AccountPurpose, WalletWrite,
    },
    proto::compact_formats::{
        ChainMetadata, CompactBlock, CompactSaplingOutput, CompactTx,
    },
};
use zcash_client_memory::MemoryWalletDb;
use zcash_keys::keys::UnifiedSpendingKey;
use zcash_protocol::consensus::{BlockHeight, MainNetwork};

struct OneBlock(CompactBlock);
impl BlockSource for OneBlock {
    type Error = ();
    fn with_blocks<F, E>(
        &self,
        _from: Option<BlockHeight>,
        _limit: Option<usize>,
        mut f: F,
    ) -> Result<(), zcash_client_backend::data_api::chain::error::Error<E, ()>>
    where
        F: FnMut(CompactBlock) -> Result<(), zcash_client_backend::data_api::chain::error::Error<E, ()>>,
    {
        f(self.0.clone())
    }
}

fn run() {
    let params = MainNetwork;
    let height = BlockHeight::from_u32(3_000_000);
    let prior = ChainState::empty(height - 1, zcash_primitives::block::BlockHash([0u8; 32]));

    let mut db = MemoryWalletDb::new(params, 100);
    let usk = UnifiedSpendingKey::from_seed(&params, &[7u8; 32], zip32::AccountId::ZERO).unwrap();
    let ufvk = usk.to_unified_full_viewing_key();
    let birthday = AccountBirthday::from_parts(prior.clone(), None);
    db.import_account_ufvk(
        "repro",
        &ufvk,
        &birthday,
        AccountPurpose::ViewOnly,
        None,
    )
    .unwrap();

    // One block with one Sapling output: forces one batch of trial decryptions through rayon.
    let mut epk = [0u8; 32];
    epk[0] = 1; // identity point
    let block = CompactBlock {
        height: u64::from(u32::from(height)),
        hash: vec![1; 32],
        prev_hash: vec![0; 32],
        time: 1,
        header: vec![],
        vtx: vec![CompactTx {
            index: 0,
            txid: vec![9; 32],
            outputs: vec![CompactSaplingOutput {
                cmu: vec![0; 32],
                ephemeral_key: epk.to_vec(),
                ciphertext: vec![0; 52],
            }],
            ..Default::default()
        }],
        chain_metadata: Some(ChainMetadata {
            sapling_commitment_tree_size: 1,
            orchard_commitment_tree_size: 0,
            ..Default::default()
        }),
    };

    let r = scan_cached_blocks(&params, &OneBlock(block), &mut db, height, &prior, 1);
    println!("scan returned: {}", if r.is_ok() { "Ok" } else { "Err" });
    if let Err(_e) = r {
        eprintln!("(scan error, not a hang)");
    }
}

fn main() {
    let pool = std::env::var("POOL").unwrap_or_else(|_| "default".into());
    if pool == "current" {
        rayon::ThreadPoolBuilder::new()
            .num_threads(1)
            .use_current_thread()
            .build_global()
            .unwrap();
    }
    let secs: u64 = std::env::var("TIMEOUT_SECS").ok().and_then(|v| v.parse().ok()).unwrap_or(20);
    let (tx, rx) = mpsc::channel();
    // The scan runs on this (the "current") thread; a watchdog thread reports the hang.
    std::thread::spawn(move || {
        if rx.recv_timeout(Duration::from_secs(secs)).is_err() {
            eprintln!("HANG: scan_cached_blocks did not return within {secs}s (POOL={pool})");
            std::process::exit(2);
        }
    });
    run();
    let _ = tx.send(());
}
