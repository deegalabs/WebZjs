//! Drives zcash_client_backend 0.25.0 `sync::run` against a fake lightwalletd
//! (an in-process tower Service) with the same store WebZjs pins (zcash_client_memory b4c8f0d).
use std::{
    convert::Infallible,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
};

use bytes::{Buf, BufMut, Bytes, BytesMut};
use http::{HeaderMap, HeaderValue, Request, Response};
use http_body::Frame;
use http_body_util::{BodyExt, StreamBody};
use prost::Message;
use tonic::body::Body as TonicBody;
use zcash_client_backend::{
    data_api::{
        chain::ChainState, wallet::ConfirmationsPolicy, AccountBirthday, AccountPurpose,
        WalletRead, WalletWrite,
    },
    proto::{
        compact_formats::{ChainMetadata, CompactBlock},
        service::{
            compact_tx_streamer_client::CompactTxStreamerClient, BlockId, BlockRange, TreeState,
        },
    },
    sync::run,
};
use zcash_client_memory::{MemBlockCache, MemoryWalletDb};
use zcash_keys::keys::UnifiedSpendingKey;
use zcash_primitives::block::BlockHash;
use zcash_protocol::consensus::{BlockHeight, MainNetwork};

const BIRTHDAY: u32 = 3_500_000;
const TIP: u32 = BIRTHDAY + 24_999; // 25,000 blocks to scan
const BATCH: u32 = 10_000;
const STORE: &str = "zcash_client_memory b4c8f0d (wallet) + MemBlockCache";

fn hash(h: u32) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, c) in out.chunks_mut(4).enumerate() {
        c.copy_from_slice(&(h ^ (i as u32).wrapping_mul(0x9e37_79b9)).to_le_bytes());
    }
    out
}

fn block(h: u32) -> CompactBlock {
    CompactBlock {
        height: h as u64,
        hash: hash(h).to_vec(),
        prev_hash: hash(h - 1).to_vec(),
        time: 1_700_000_000 + h,
        header: vec![],
        vtx: vec![],
        chain_metadata: Some(ChainMetadata {
            sapling_commitment_tree_size: 0,
            orchard_commitment_tree_size: 0,
            ironwood_commitment_tree_size: 0,
        }),
    }
}

#[derive(Clone, Default)]
struct Cfg {
    /// the n-th GetBlockRange call (1-based) returns only this many blocks, then an OK trailer
    short_on_call: Option<(usize, usize)>,
    /// every GetBlockRange returns at most this many blocks, then an OK trailer
    cap_always: Option<usize>,
    /// the n-th GetBlockRange call sends this many blocks, then a trailer with grpc-status 11
    error_on_call: Option<(usize, usize)>,
    /// this height is never sent
    omit_height: Option<u32>,
    /// stop the experiment with an error after this many GetBlockRange calls
    max_calls: Option<usize>,
}

#[derive(Default)]
struct State {
    cfg: Cfg,
    range_calls: Vec<(u32, u32, usize, &'static str)>,
}

#[derive(Clone)]
struct Fake(Arc<Mutex<State>>);

type RespBody = StreamBody<futures_util::stream::Iter<std::vec::IntoIter<Result<Frame<Bytes>, Infallible>>>>;

fn frame_msg<M: Message>(m: &M) -> Bytes {
    let mut b = BytesMut::new();
    b.put_u8(0);
    b.put_u32(m.encoded_len() as u32);
    m.encode(&mut b).unwrap();
    b.freeze()
}

fn respond(msgs: Vec<Bytes>, status: u32, message: &str) -> Response<RespBody> {
    let mut frames: Vec<Result<Frame<Bytes>, Infallible>> =
        msgs.into_iter().map(|b| Ok(Frame::data(b))).collect();
    let mut t = HeaderMap::new();
    t.insert("grpc-status", HeaderValue::from_str(&status.to_string()).unwrap());
    if !message.is_empty() {
        t.insert("grpc-message", HeaderValue::from_str(message).unwrap());
    }
    frames.push(Ok(Frame::trailers(t)));
    Response::builder()
        .status(200)
        .header("content-type", "application/grpc")
        .body(StreamBody::new(futures_util::stream::iter(frames)))
        .unwrap()
}

impl tower_service::Service<Request<TonicBody>> for Fake {
    type Response = Response<RespBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Infallible>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Infallible>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<TonicBody>) -> Self::Future {
        let st = self.0.clone();
        Box::pin(async move {
            let path = req.uri().path().to_string();
            let mut body = req.into_body().collect().await.unwrap().to_bytes();
            if body.len() >= 5 {
                body.advance(5);
            }
            let method = path.rsplit('/').next().unwrap().to_string();
            Ok(match method.as_str() {
                "GetLatestBlock" => respond(
                    vec![frame_msg(&BlockId { height: TIP as u64, hash: hash(TIP).to_vec() })],
                    0,
                    "",
                ),
                "GetTreeState" => {
                    let id = BlockId::decode(body).unwrap();
                    let mut hb = hash(id.height as u32);
                    hb.reverse();
                    respond(
                        vec![frame_msg(&TreeState {
                            network: "main".into(),
                            height: id.height,
                            hash: hex::encode(hb),
                            time: 0,
                            sapling_tree: "000000".into(),
                            orchard_tree: "000000".into(),
                            ironwood_tree: "000000".into(),
                        })],
                        0,
                        "",
                    )
                }
                "GetBlockRange" => {
                    let r = BlockRange::decode(body).unwrap();
                    let (s, e) = (r.start.unwrap().height as u32, r.end.unwrap().height as u32);
                    let mut st = st.lock().unwrap();
                    let n = st.range_calls.len() + 1;
                    let cfg = st.cfg.clone();
                    if cfg.max_calls.map_or(false, |m| n > m) {
                        st.range_calls.push((s, e, 0, "experiment stop"));
                        return Ok(respond(vec![], 10, "experiment stop"));
                    }
                    let mut heights: Vec<u32> =
                        (s..=e).filter(|h| Some(*h) != cfg.omit_height).collect();
                    let mut status = 0;
                    let mut note = "ok";
                    if let Some(cap) = cfg.cap_always {
                        if heights.len() > cap {
                            heights.truncate(cap);
                            note = "SHORT, OK trailer";
                        }
                    }
                    if let Some((call, keep)) = cfg.short_on_call {
                        if call == n {
                            heights.truncate(keep);
                            note = "SHORT, OK trailer";
                        }
                    }
                    if let Some((call, keep)) = cfg.error_on_call {
                        if call == n {
                            heights.truncate(keep);
                            status = 11;
                            note = "blocks then grpc-status 11 trailer";
                        }
                    }
                    st.range_calls.push((s, e, heights.len(), note));
                    respond(
                        heights.iter().map(|h| frame_msg(&block(*h))).collect(),
                        status,
                        if status == 0 { "" } else { "out of range" },
                    )
                }
                // GetSubtreeRoots, GetAddressUtxosStream, anything else: an empty OK stream
                _ => respond(vec![], 0, ""),
            })
        })
    }
}

fn new_wallet() -> MemoryWalletDb<MainNetwork> {
    let mut db = MemoryWalletDb::new(MainNetwork, 100);
    let usk = UnifiedSpendingKey::from_seed(&MainNetwork, &[7u8; 32], zip32::AccountId::ZERO).unwrap();
    let ufvk = usk.to_unified_full_viewing_key();
    let birthday = AccountBirthday::from_parts(
        ChainState::empty(BlockHeight::from(BIRTHDAY - 1), BlockHash(hash(BIRTHDAY - 1))),
        None,
    );
    db.import_account_ufvk("a", &ufvk, &birthday, AccountPurpose::ViewOnly, None).unwrap();
    db
}

fn report(db: &MemoryWalletDb<MainNetwork>) {
    let mut buf = Vec::new();
    db.encode(&mut buf).unwrap();
    println!("    encoded store: {} bytes", buf.len());
    let ranges = db.suggest_scan_ranges().unwrap();
    println!("    suggest_scan_ranges after: {} range(s)", ranges.len());
    for r in &ranges {
        println!("      {:?} len {}", r, r.len());
    }
    println!(
        "    chain_height {:?}  block_fully_scanned {:?}  block_max_scanned {:?}",
        db.chain_height().unwrap(),
        db.block_fully_scanned().unwrap().map(|m| m.block_height()),
        db.block_max_scanned().unwrap().map(|m| m.block_height()),
    );
    match db.get_wallet_summary(ConfirmationsPolicy::MIN) {
        Ok(Some(s)) => println!(
            "    summary: chain_tip {:?} fully_scanned {:?} progress scan {:?} recovery {:?}",
            s.chain_tip_height(),
            s.fully_scanned_height(),
            s.progress().scan(),
            s.progress().recovery()
        ),
        other => println!("    summary: {:?}", other.map(|_| ())),
    }
}

async fn sync_once(db: &mut MemoryWalletDb<MainNetwork>, st: &Arc<Mutex<State>>, label: &str) {
    st.lock().unwrap().range_calls.clear();
    let mut client = CompactTxStreamerClient::new(Fake(st.clone()));
    // WebZjs: a fresh MemBlockCache per sync() (wallet.rs:286)
    let cache = MemBlockCache::new();
    let res = tokio::time::timeout(
        std::time::Duration::from_secs(120),
        run(&mut client, &MainNetwork, &cache, db, BATCH),
    )
    .await;
    match res {
        Ok(Ok(())) => println!("  {label}: run() -> Ok(())"),
        Ok(Err(e)) => println!("  {label}: run() -> Err({e})"),
        Err(_) => println!("  {label}: run() DID NOT RETURN in 120 s"),
    }
    let calls = st.lock().unwrap().range_calls.clone();
    let shown = calls.len().min(12);
    for (s, e, n, note) in &calls[..shown] {
        println!("    GetBlockRange {s}..={e} (asked {}) -> sent {n} [{note}]", e - s + 1);
    }
    if calls.len() > shown {
        println!("    ... {} GetBlockRange calls in total", calls.len());
    }
    println!(
        "    blocks left in the cache: {}",
        cache_len(&cache)
    );
    report(db);
}

fn cache_len(cache: &MemBlockCache) -> usize {
    (BIRTHDAY - 20..=TIP).filter(|h| cache.find_block(BlockHeight::from(*h)).is_some()).count()
}

async fn scenario(name: &str, cfg: Cfg, second_cfg: Option<Cfg>) {
    println!("\n=== {name} ===");
    let st = Arc::new(Mutex::new(State { cfg, range_calls: vec![] }));
    let mut db = new_wallet();
    sync_once(&mut db, &st, "sync #1").await;
    if let Some(c) = second_cfg {
        st.lock().unwrap().cfg = c;
        sync_once(&mut db, &st, "sync #2").await;
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    println!("store: {STORE}");
    println!("birthday {BIRTHDAY}, tip {TIP}, batch {BATCH}");
    let only = std::env::args().nth(1);
    let want = |k: &str| only.as_deref().map_or(true, |o| o == k);
    if want("B") {
        scenario(
            "B. 1st GetBlockRange ends early (500 of 10000) with an OK trailer; honest afterwards",
            Cfg { short_on_call: Some((1, 500)), ..Default::default() },
            Some(Cfg::default()),
        )
        .await;
    }
    if want("E") {
        scenario(
            "E. 2nd GetBlockRange sends 300 blocks then a grpc-status 11 trailer; honest afterwards",
            Cfg { error_on_call: Some((2, 300)), ..Default::default() },
            Some(Cfg::default()),
        )
        .await;
    }
    if want("F") {
        scenario(
            "F. server always omits one block in the FIRST batch; stop after 60 calls",
            Cfg { omit_height: Some(BIRTHDAY + 4_000), max_calls: Some(60), ..Default::default() },
            None,
        )
        .await;
    }
    if want("F2") {
        scenario(
            "F2. server always omits one block in the second batch; stop after 60 calls",
            Cfg { omit_height: Some(BIRTHDAY + 13_991), max_calls: Some(60), ..Default::default() },
            None,
        )
        .await;
    }
    if only.as_deref() == Some("F2-nocap") {
        scenario(
            "F2-nocap. as F2, but the server never stops the experiment; run() is given 120 s",
            Cfg { omit_height: Some(BIRTHDAY + 13_991), ..Default::default() },
            None,
        )
        .await;
    }
}
