//! RED TEAM: cUSD, the faucet and griefing. Claim tricks, draining the faucet, blocking other people's
//! claims, unfillable or stuck orders, blocking fills and cancels. Each test records who is harmed.

mod common;

use common::*;
use tari_template_lib::prelude::{Amount, ComponentAddress, OwnerRule, ResourceAddress, TARI_TOKEN};
use tari_template_test_tooling::TemplateTest;
use tari_template_test_tooling::transaction::args;

const N: u64 = 100 * T;
const P: u64 = 50 * T;

fn world() -> World {
    let mut test = TemplateTest::new(env!("CARGO_MANIFEST_DIR"), [
        "../templates/cusd_faucet",
        "../templates/board",
        "../templates/order",
        "fixtures/burn_wallet",
        "fixtures/board_v2",
        "fixtures/mint_attempt",
        "fixtures/attack_escrow",
        "fixtures/attack_grief",
    ]);
    let faucet: ComponentAddress = test.call_function("CaravelTestDollar", "new", args![], vec![]);
    let cusd: ResourceAddress = test.call_method(faucet, "resource", args![], vec![]);
    let burn_wallet: ComponentAddress = test.call_function("CaravelBurnWallet", "new", args![], vec![]);
    let caravel = party(&mut test);
    let owner = party(&mut test);
    let order_template = test.get_template_address("CaravelTradeOrder");
    let board: ComponentAddress = test.call_function(
        "CaravelTradeBoard",
        "new",
        args![OwnerRule::ByPublicKey(owner.pk), order_template, cusd, caravel.account, burn_wallet],
        vec![],
    );
    World { test, board, faucet, cusd, burn_wallet, caravel, owner }
}

fn grief(w: &World) -> tari_template_lib::prelude::TemplateAddress {
    w.test.get_template_address("CaravelAttackGrief")
}

fn remaining(w: &mut World) -> Amount {
    w.test.call_method(w.faucet, "remaining", args![], vec![])
}

// ── Claims ───────────────────────────────────────────────────────────────────

#[test]
fn a_claim_through_another_template_is_refused() {
    // The signer badge only reaches the top-level call, so a template cannot claim for its caller.
    let mut w = world();
    let alice = w.party();
    let b = w.tx().call_function(grief(&w), "claim_via", args![w.faucet, alice.pk]).put_last_instruction_output_on_workspace("c")
        .call_method(alice.account, "deposit", args![Workspace("c")]);
    let o = w.run(b, &alice, &[]);
    println!("claim via a template: {}", o.error);
    assert!(!o.ok());
    w.claim_to_account(&alice).expect_ok("alice's own claim still works");
}

#[test]
fn two_claims_in_one_transaction_are_refused() {
    let mut w = world();
    let alice = w.party();
    let b = w.tx()
        .call_method(w.faucet, "claim", args![alice.pk]).put_last_instruction_output_on_workspace("c1")
        .call_method(w.faucet, "claim", args![alice.pk]).put_last_instruction_output_on_workspace("c2")
        .call_method(alice.account, "deposit", args![Workspace("c1")])
        .call_method(alice.account, "deposit", args![Workspace("c2")]);
    w.run(b, &alice, &[]).expect_err("double claim", "Duplicate");
    assert_eq!(w.balance(alice.account, w.cusd), Amount::zero());
}

#[test]
fn nobody_can_block_a_wallets_claim_not_even_the_deployer() {
    // ATTACK: mint-and-burn a victim's claim receipt before they claim, so their own claim fails as a
    // duplicate. The receipt resource has no owner, so only the faucet component passes its mint rule.
    let mut w = world();
    let victim = w.party();
    let receipts: ResourceAddress = w.test.extract_component_value(w.faucet, "$.1");
    let owner = format!("{:?}", w.test.read_only_state_store().get_resource(&receipts).unwrap().owner_rule());
    assert!(owner.contains("None"), "the receipt resource must have no owner: {owner}");

    // A stranger.
    let stranger = w.party();
    let o = w.run(w.tx().call_function(grief(&w), "burn_receipt_of", args![receipts, victim.pk]), &stranger, &[]);
    o.expect_err("a stranger pre-burns a receipt", "Access Denied");

    // The faucet's deployer (the harness's default key deployed it).
    let tx = w.tx().call_function(grief(&w), "burn_receipt_of", args![receipts, victim.pk]).finish().seal(w.test.secret_key());
    w.exec(tx).expect_err("the deployer pre-burns a receipt", "Access Denied");

    // The victim's claim still works, and only once.
    w.claim_to_account(&victim).expect_ok("the victim's claim");
    assert_eq!(w.balance(victim.account, w.cusd), Amount::from(10_000 * T));
    w.claim_to_account(&victim).expect_err("a second claim", "Duplicate");
}

// ── Unfillable and stuck orders ──────────────────────────────────────────────

#[test]
fn an_order_paying_into_a_refusing_account_cannot_fill_but_its_maker_can_cancel() {
    // GRIEF (self, and takers' network fees): a maker names a payout account that refuses deposits,
    // or the Burn Wallet (which refuses cUSD). No fill can succeed; nothing gets stuck.
    let mut w = world();
    let refusing: ComponentAddress = w.test.call_function("HostileAccount", "new", args![3u8], vec![]);
    for account in [refusing, w.burn_wallet] {
        let maker = w.party();
        let taker = w.party();
        let (cancel_s, cancel) = keypair();
        let b = w.tx()
            .call_method(maker.account, "withdraw", args![TARI_TOKEN, Amount::from(N)]).put_last_instruction_output_on_workspace("e")
            .call_method(w.board, "place_sell_public", args![Workspace("e"), Amount::from(P), account, cancel]);
        let o = w.run(b, &maker, &[]);
        o.expect_ok("place");
        let order = w.order_in(&o);
        assert!(!w.fill(&taker, order, Funds::Public, Funds::Public).0.ok(), "a fill paid into a refusing account");
        assert_eq!(w.status(order), OPEN);
        let m0 = w.balance(maker.account, TARI_TOKEN);
        w.cancel(&maker, &cancel_s, order).expect_ok("cancel");
        assert_eq!(w.balance(maker.account, TARI_TOKEN) - m0, Amount::from(N));
    }
}

#[test]
fn an_order_whose_cancel_key_is_lost_is_still_fillable_so_nothing_is_stuck() {
    let mut w = world();
    let maker = w.party();
    let taker = w.party();
    let (_lost, cancel) = keypair();
    let placed = w.place(&maker, SELL, N, P, Funds::Public, Funds::Private, cancel);
    let (o, _) = w.fill(&taker, placed.order, Funds::Public, Funds::Public);
    o.expect_ok("fill");
    assert!(o.created(placed.payout.as_ref().unwrap()), "the maker is still paid");
}

#[test]
fn trust_assumption_caravel_can_stop_fills_by_breaking_its_own_fee_account() {
    // DOCUMENTED, NOT A VULNERABILITY: every order snapshots the Caravel fee account. That account's
    // OWNER (Caravel) can switch it to another template; if its `deposit` then fails, no order with a
    // Caravel share can fill. Cancels still work, so no funds are stuck. The Burn Wallet has no owner
    // and cannot be switched.
    let mut w = world();
    let maker = w.party();
    let taker = w.party();
    let (cancel_s, cancel) = keypair();
    let a = w.place(&maker, SELL, N, P, Funds::Public, Funds::Public, cancel).order;
    let b = w.place(&maker, BUY, N, P, Funds::Public, Funds::Public, cancel).order;
    let hostile = w.test.get_template_address("HostileAccount");
    let caravel = Party { account: w.caravel.account, secret: w.caravel.secret.clone(), pk: w.caravel.pk };
    let o = w.run(w.tx().update_component_template(caravel.account, hostile), &caravel, &[]);
    println!("Caravel switches its fee account's template: {}", if o.ok() { "ALLOWED".to_string() } else { o.error.clone() });
    if o.ok() {
        let (f, _) = w.fill(&taker, a, Funds::Public, Funds::Public);
        println!("fill afterwards: {}", f.error);
        assert!(!f.ok(), "documented: fills stop");
        w.cancel(&maker, &cancel_s, b).expect_ok("cancels still work");
        w.cancel(&maker, &cancel_s, a).expect_ok("cancels still work");
    }
    // Nobody else can switch it, and nobody can switch the Burn Wallet.
    let stranger = w.party();
    assert!(!w.run(w.tx().update_component_template(w.burn_wallet, hostile), &stranger, &[]).ok());
}

#[test]
fn dust_orders_cost_their_maker_a_full_tari_each() {
    // SPAM: the cheapest order locks 1 tTARI (the minimum) until cancelled; below it is refused.
    let mut w = world();
    let spammer = w.party();
    let (_, cancel) = keypair();
    w.place(&spammer, SELL, T - 1, 1, Funds::Public, Funds::Public, cancel).outcome.expect_err("dust", "Below the minimum");
    w.place(&spammer, BUY, T - 1, 1, Funds::Public, Funds::Public, cancel).outcome.expect_err("dust", "Below the minimum");
}
