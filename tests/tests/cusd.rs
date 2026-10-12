//! cUSD and its faucet: fixed supply, nothing can mint, burn, recall or freeze it, 10,000 per claim,
//! once per wallet, only for the claimer's own key.

mod common;

use common::*;
use tari_template_lib::prelude::{Amount, ResourceAddress, VaultId};
use tari_template_lib::types::access_rules::{AccessRule, ResourceAuthAction};
use tari_template_test_tooling::transaction::args;

const SUPPLY: u64 = 1_000_000_000 * T;
const CLAIM: u64 = 10_000 * T;

#[test]
fn a_claim_pays_10_000_cusd_into_the_claimers_hands() {
    let mut w = world();
    let alice = w.party();
    w.claim_to_account(&alice).expect_ok("claim");
    assert_eq!(w.balance(alice.account, w.cusd), Amount::from(CLAIM));
    let remaining: Amount = w.test.call_method(w.faucet, "remaining", args![], vec![]);
    assert_eq!(remaining, Amount::from(SUPPLY - CLAIM));
}

#[test]
fn a_claim_can_be_made_private_in_the_same_transaction() {
    let mut w = world();
    let alice = w.party();
    let r = receive(CLAIM);
    let b = w.tx()
        .call_method(w.faucet, "claim", args![alice.pk])
        .put_last_instruction_output_on_workspace("c")
        .stealth_transfer_with_input_bucket(w.cusd, r.clone(), "c");
    let o = w.run(b, &alice, &[]);
    o.expect_ok("private claim");
    assert!(o.created(&r), "the claimer's private cUSD output was not created");
    assert_eq!(w.balance(alice.account, w.cusd), Amount::zero(), "nothing should touch the public account");
}

#[test]
fn one_claim_per_wallet() {
    let mut w = world();
    let alice = w.party();
    w.claim_to_account(&alice).expect_ok("first claim");
    w.claim_to_account(&alice).expect_err("second claim", "Duplicate");
    assert_eq!(w.balance(alice.account, w.cusd), Amount::from(CLAIM));
    // Someone else still can.
    let bob = w.party();
    w.claim_to_account(&bob).expect_ok("another wallet's claim");
}

#[test]
fn nobody_can_claim_for_someone_elses_key() {
    let mut w = world();
    let thief = w.party();
    let victim = w.party();
    let b = w.tx()
        .call_method(w.faucet, "claim", args![victim.pk])
        .put_last_instruction_output_on_workspace("c")
        .call_method(thief.account, "deposit", args![Workspace("c")]);
    let o = w.run(b, &thief, &[]);
    assert!(!o.ok(), "claiming for another key must fail");
    w.claim_to_account(&victim).expect_ok("the victim can still claim");
}

#[test]
fn the_supply_is_fixed_and_nobody_owns_it() {
    let mut w = world();
    let resource = w.test.read_only_state_store().get_resource(&w.cusd).unwrap();
    assert_eq!(resource.total_supply(), Some(Amount::from(SUPPLY)));
    let rules = resource.access_rules();
    for action in [ResourceAuthAction::Mint, ResourceAuthAction::Burn, ResourceAuthAction::Recall, ResourceAuthAction::Freeze] {
        assert_eq!(*rules.get_access_rule(&action), AccessRule::DenyAll, "{action:?} must be denied");
    }
    assert!(format!("{:?}", resource.owner_rule()).contains("None"), "cUSD must have no owner: {:?}", resource.owner_rule());

    let attacker = w.party();
    let faucet_vault: VaultId = w.test.extract_component_value(w.faucet, "$.0");
    let tpl = w.test.get_template_address("CaravelMintAttempt");
    let cusd: ResourceAddress = w.cusd;

    let attempts = [
        ("mint", w.tx().call_function(tpl, "mint", args![cusd]).put_last_instruction_output_on_workspace("m").call_method(attacker.account, "deposit", args![Workspace("m")])),
        ("recall", w.tx().call_function(tpl, "recall", args![cusd, faucet_vault]).put_last_instruction_output_on_workspace("m").call_method(attacker.account, "deposit", args![Workspace("m")])),
        ("freeze", w.tx().call_function(tpl, "freeze", args![cusd, faucet_vault])),
        ("open minting", w.tx().call_function(tpl, "open_minting", args![cusd])),
    ];
    for (what, b) in attempts {
        let o = w.run(b, &attacker, &[]);
        assert!(!o.ok(), "{what} must be refused");
        println!("{what}: {}", o.error);
    }

    // Burning claimed cUSD is refused too.
    w.claim_to_account(&attacker).expect_ok("claim");
    let b = w.tx()
        .call_method(attacker.account, "withdraw", args![cusd, Amount::from(T)])
        .put_last_instruction_output_on_workspace("b")
        .call_function(tpl, "burn", args![Workspace("b")]);
    let o = w.run(b, &attacker, &[]);
    assert!(!o.ok(), "burn must be refused");
    println!("burn: {}", o.error);

    let resource = w.test.read_only_state_store().get_resource(&w.cusd).unwrap();
    assert_eq!(resource.total_supply(), Some(Amount::from(SUPPLY)), "the supply moved");
}

#[test]
fn the_faucet_has_no_admin_surface() {
    let mut w = world();
    let anyone = w.party();
    // No method withdraws or changes anything; an unknown method is refused.
    let o = w.run(w.tx().call_method(w.faucet, "withdraw", args![Amount::from(T)]), &anyone, &[]);
    assert!(!o.ok());
}
