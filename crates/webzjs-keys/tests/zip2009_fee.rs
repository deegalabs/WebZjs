// Copyright 2024 ChainSafe Systems
// SPDX-License-Identifier: Apache-2.0, MIT

//! ZIP 2009 lowers the ZIP 317 marginal fee from 5,000 to 1,000 zatoshis per
//! logical action from Mainnet block 3,590,000 and Testnet block 4,465,026 (the
//! fixtures use the test network). With the ZIP 317 grace of two actions, the
//! minimum fee becomes 2,000. A PCZT that pays it must validate.
//! Fixture: tests/fixtures/zip2009_fee_2000.pczt, built by `generate_fixtures`
//! in gen_fixtures.rs (spend 100_000 of our Ironwood note, pay 98_000, fee
//! 2_000; one logical action: the extra action the builder adds is not
//! counted).
//!
//! Run: `cargo test -p webzjs-keys --test zip2009_fee`

use std::str::FromStr;
use webzjs_common::Network;
use webzjs_keys::{pczt_describe_inner, pczt_validate_inner};

/// Must match `OUR_SEED` in gen_fixtures.rs.
const TEST_SEED: &[u8; 32] = &[0x01; 32];

fn our_ufvk() -> zcash_keys::keys::UnifiedFullViewingKey {
    let network = Network::from_str("test").expect("valid network");
    zcash_keys::keys::UnifiedSpendingKey::from_seed(&network, TEST_SEED, zip32::AccountId::try_from(0).unwrap())
        .expect("derive USK from seed")
        .to_unified_full_viewing_key()
}

fn load(name: &str) -> pczt::Pczt {
    let path = format!("{}/tests/fixtures/{name}.pczt", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    pczt::Pczt::parse(&bytes).unwrap_or_else(|e| panic!("parse {path}: {e:?}"))
}

#[test]
fn a_pczt_paying_the_zip2009_minimum_fee_validates() {
    let ufvk = our_ufvk();
    let summary = pczt_describe_inner(Network::from_str("test").unwrap(), load("zip2009_fee_2000"), &ufvk)
        .expect("describe");
    assert_eq!(summary.fee, 2_000, "the fixture pays the ZIP 2009 minimum fee");

    let result = pczt_validate_inner(Network::from_str("test").unwrap(), load("zip2009_fee_2000"), &ufvk);
    assert!(
        result.is_ok(),
        "a PCZT paying the ZIP 2009 minimum fee (2,000: one logical action, two-action grace) must validate, got: {:?}",
        result.err().map(|e| e.to_string())
    );
}
