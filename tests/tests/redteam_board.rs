//! RED TEAM: the Board and trust. Fake orders, badge forgery, owner and permission tricks, Board
//! upgrades aimed at existing orders, pause abuse. Each test asserts the attack does no harm, or, for
//! what a malicious Board OWNER can do, documents it as a trust assumption.

mod common;

use common::*;
use tari_template_lib::prelude::{Amount, ComponentAddress, OwnerRule, ResourceAddress, TARI_TOKEN};
use tari_template_test_tooling::transaction::args;

const N: u64 = 100 * T;
const P: u64 = 50 * T;

/// The common world plus the two attacker fixtures. Built here so the shared helpers stay untouched.
fn world() -> World {
    let mut test = template_test([
        "../templates/cusd_faucet",
        "../templates/board",
        "../templates/order",
        "fixtures/burn_wallet",
        "fixtures/board_v2",
        "fixtures/mint_attempt",
        "fixtures/attack_board_hostile",
        "fixtures/attack_board_caller",
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

fn badge(w: &World) -> ResourceAddress {
    w.test.extract_component_value(w.board, "$.9")
}

/// The legitimacy check an app runs on a verified read of an order (the published rule).
fn looks_legit(w: &World, order: ComponentAddress) -> bool {
    let store = w.test.read_only_state_store();
    let c = store.get_component(order).unwrap();
    let template_ok = *c.template_address() == w.test.get_template_address("CaravelTradeOrder");
    let no_owner = format!("{:?}", c.owner_rule()).contains("None");
    let board: ComponentAddress = w.test.extract_component_value(order, "$.2");
    let order_badge: ResourceAddress = w.test.extract_component_value(order, "$.3");
    template_ok && no_owner && board == w.board && order_badge == badge(w) && w.status(order) == OPEN
}

fn caller(w: &World) -> tari_template_lib::prelude::TemplateAddress {
    w.test.get_template_address("CaravelAttackBoardCaller")
}

// ── Fake orders ──────────────────────────────────────────────────────────────

#[test]
fn a_forged_order_naming_the_real_board_is_created_but_fails_the_legitimacy_check() {
    // ATTACK: call the genuine Order template directly with a look-alike badge (same symbol) and the
    // real Board's address in `board`. The engine lets anyone create an Order component this way.
    let mut w = world();
    let forger = w.party();
    let (_, cancel) = keypair();
    // Step 1: mint a look-alike badge (symbol CTBOARD, supply 1) into the forger's account.
    let b = w.tx().call_function(caller(&w), "fake_badge", args![]).put_last_instruction_output_on_workspace("fake")
        .call_method(forger.account, "deposit", args![Workspace("fake")]);
    let o = w.run(b, &forger, &[]);
    o.expect_ok("fake badge");
    let fake: ResourceAddress = o.ups.iter().filter(|id| id.starts_with("resource_")).next().unwrap().parse().unwrap();
    // Step 2: call the genuine Order template at the top level with a proof of the look-alike.
    let b = w.tx()
        .create_proof(forger.account, fake).put_last_instruction_output_on_workspace("proof")
        .call_method(forger.account, "withdraw", args![TARI_TOKEN, Amount::from(N)]).put_last_instruction_output_on_workspace("e")
        .call_function(w.test.get_template_address("CaravelTradeOrder"), "create", args![
            Workspace("proof"), fake, w.board, SELL, Workspace("e"), Amount::from(N), Amount::from(P), w.cusd,
            0u16, 0u8, forger.account, forger.account, tari_template_lib::types::bytes::Bytes::default(), vec![forger.account], cancel
        ]);
    let o = w.run(b, &forger, &[]);
    o.expect_ok("the forged order is created (anyone may call the Order template)");
    let forged = w.order_in(&o);
    let board_field: ComponentAddress = w.test.extract_component_value(forged, "$.2");
    assert_eq!(board_field, w.board, "the forged order does carry the real Board's address");
    // HARMLESS IF THE APP CHECKS THE BADGE: the badge field is the look-alike, not the real one.
    assert!(!looks_legit(&w, forged), "a forged order passed the legitimacy check");
    // Control: a real order passes.
    let maker = w.party();
    let real = w.place(&maker, SELL, N, P, Funds::Public, Funds::Public, cancel).order;
    assert!(looks_legit(&w, real));
}

#[test]
fn a_forged_order_cannot_carry_the_real_badge() {
    // ATTACK: Order::create naming the REAL badge, with a proof of something else.
    let mut w = world();
    let forger = w.party();
    w.claim_to_account(&forger).expect_ok("claim");
    let (_, bytes) = payout(P);
    let (_, cancel) = keypair();
    let b = w.tx()
        .create_proof(forger.account, w.cusd).put_last_instruction_output_on_workspace("proof")
        .call_method(forger.account, "withdraw", args![TARI_TOKEN, Amount::from(N)]).put_last_instruction_output_on_workspace("e")
        .call_function(w.test.get_template_address("CaravelTradeOrder"), "create", args![
            Workspace("proof"), badge(&w), w.board, SELL, Workspace("e"), Amount::from(N), Amount::from(P), w.cusd,
            0u16, 0u8, forger.account, forger.account, bytes, Vec::<ComponentAddress>::new(), cancel
        ]);
    w.run(b, &forger, &[]).expect_err("forged order with the real badge", "Proof of resource did not match");
}

#[test]
fn nobody_but_the_board_can_prove_the_badge() {
    // ATTACK: the badge is a public fungible; try to get one. Its supply is 1 and it sits in the
    // Board's vault; nobody holds a bucket of it, and the Board has no method that hands it out.
    let mut w = world();
    let thief = w.party();
    let res = w.test.read_only_state_store().get_resource(&badge(&w)).unwrap();
    assert_eq!(res.total_supply(), Some(Amount::from(1u64)));
    let o = w.run(w.tx().create_proof(thief.account, badge(&w)).put_last_instruction_output_on_workspace("p"), &thief, &[]);
    assert!(!o.ok(), "a stranger produced a badge proof");
    let tpl = w.test.get_template_address("CaravelMintAttempt");
    let o = w.run(w.tx().call_function(tpl, "mint", args![badge(&w)]).put_last_instruction_output_on_workspace("m")
        .call_method(thief.account, "deposit", args![Workspace("m")]), &thief, &[]);
    assert!(!o.ok(), "minting a second badge must fail: {}", o.error);
}

// ── Permissions ──────────────────────────────────────────────────────────────

#[test]
fn owner_methods_are_refused_through_another_template() {
    // ATTACK: reach owner-only methods through a component call instead of a signed instruction.
    let mut w = world();
    let attacker = w.party();
    let board = w.board;
    for method in ["set_fee", "pause"] {
        let o = w.run(w.tx().call_function(caller(&w), method, args![board]), &attacker, &[]);
        o.expect_err(method, "Access Denied");
    }
    // Even when the OWNER signs, the call made by another template is not the owner's.
    let b = w.tx().call_function(caller(&w), "set_fee", args![board]);
    let o = w.as_owner(b);
    println!("owner-signed tx, call via another template: {}", if o.ok() { "ALLOWED".into() } else { o.error.clone() });
    let fee: u16 = w.test.extract_component_value(board, "$.2");
    if o.ok() {
        // Signer badges do not reach nested calls in this engine; if this ever passes, the owner's own
        // signature authorised it, which is not an escalation.
        assert_eq!(fee, 10_000);
    } else {
        assert_eq!(fee, 100);
    }
}

#[test]
fn a_cancel_through_another_template_is_refused_even_with_the_cancel_key_signing() {
    let mut w = world();
    let maker = w.party();
    let (cancel_s, cancel) = keypair();
    let order = w.place(&maker, SELL, N, P, Funds::Public, Funds::Public, cancel).order;
    let attacker = w.party();
    let b = w.tx().call_function(caller(&w), "cancel", args![order]).put_last_instruction_output_on_workspace("b")
        .call_method(attacker.account, "deposit", args![Workspace("b")]);
    w.run(b, &attacker, &[]).expect_err("cancel via a template", "Access Denied");
    // With the cancel key signing the transaction: the nested call still carries no signer badge.
    let b = w.tx().call_function(caller(&w), "cancel", args![order]).put_last_instruction_output_on_workspace("b")
        .call_method(attacker.account, "deposit", args![Workspace("b")]);
    let tx = b.finish().add_signer(&attacker.pk, &cancel_s).seal(&attacker.secret);
    let o = w.exec(tx);
    println!("cancel via a template with the cancel key signing: {}", if o.ok() { "ALLOWED".to_string() } else { o.error.clone() });
    // Either way the escrow only moves with the cancel key's consent.
    assert_eq!(w.status(order), if o.ok() { CANCELLED } else { OPEN });
}

#[test]
fn a_stranger_cannot_upgrade_the_board_or_any_order() {
    let mut w = world();
    let stranger = w.party();
    let maker = w.party();
    let (_, cancel) = keypair();
    let order = w.place(&maker, SELL, N, P, Funds::Public, Funds::Public, cancel).order;
    let hostile = w.test.get_template_address("CaravelTradeBoardHostile");
    w.run(w.tx().update_component_template(w.board, hostile), &stranger, &[]).expect_err("stranger upgrades the Board", "must be the owner");
    w.run(w.tx().update_component_template(order, hostile), &stranger, &[]).expect_err("stranger upgrades an order", "must be the owner");
}

// ── A malicious or compromised OWNER ─────────────────────────────────────────

#[test]
fn a_hostile_board_upgrade_cannot_reach_existing_orders() {
    let mut w = world();
    let maker = w.party();
    let taker = w.party();
    let (cancel_s, cancel) = keypair();
    let sell = w.place(&maker, SELL, N, P, Funds::Public, Funds::Private, cancel);
    let buy = w.place(&maker, BUY, N, P, Funds::Public, Funds::Public, cancel).order;
    let board = w.board;
    let hostile = w.test.get_template_address("CaravelTradeBoardHostile");
    w.as_owner(w.tx().update_component_template(board, hostile)).expect_ok("owner swaps in hostile code");

    // The hostile Board tries to cancel an existing order and keep the escrow.
    w.as_owner(w.tx().call_method(board, "evil_cancel", args![buy])).expect_err("hostile cancel", "Access Denied");
    // ...and the owner cannot switch an order's code.
    w.as_owner(w.tx().update_component_template(sell.order, hostile)).expect_err("owner upgrades an order", "must be the owner");

    // Existing orders fill and cancel on their original terms.
    let (c0, b0) = (w.caravel_tari(), w.burned());
    let (o, _) = w.fill(&taker, sell.order, Funds::Public, Funds::Public);
    o.expect_ok("fill after the hostile upgrade");
    assert!(o.created(sell.payout.as_ref().unwrap()));
    assert_eq!(w.caravel_tari() - c0, Amount::from(700_000u64));
    assert_eq!(w.burned() - b0, Amount::from(300_000u64));
    w.cancel(&maker, &cancel_s, buy).expect_ok("cancel after the hostile upgrade");
}

#[test]
fn trust_assumption_a_hostile_owner_controls_new_orders_and_the_badge() {
    // DOCUMENTED, NOT A VULNERABILITY: whoever controls the Board's owner key controls what NEW
    // orders look like. A hostile upgrade can (a) place genuine-looking orders with any terms and
    // (b) take the badge out of the Board. Neither reaches existing orders or anyone's funds: an
    // order's terms are in its state for every taker to read before filling.
    let mut w = world();
    let maker = w.party();
    let (_, cancel) = keypair();
    let board = w.board;
    let hostile = w.test.get_template_address("CaravelTradeBoardHostile");
    w.as_owner(w.tx().update_component_template(board, hostile)).expect_ok("hostile upgrade");

    let owner = Party { account: w.owner.account, secret: w.owner.secret.clone(), pk: w.owner.pk };
    let b = w.tx()
        .call_method(owner.account, "withdraw", args![TARI_TOKEN, Amount::from(N)]).put_last_instruction_output_on_workspace("e")
        .call_method(board, "evil_place_zero_fee", args![Workspace("e"), Amount::from(P), owner.account, cancel]);
    let o = w.run(b, &owner, &[]);
    o.expect_ok("(a) a zero-fee order from the hostile Board");
    let odd = w.order_in(&o);
    assert!(looks_legit(&w, odd), "(a) it passes the legitimacy check: trust in the owner is required");
    assert_eq!(w.order_fee(odd), 0, "(a) its terms are visible: fee 0");

    let b = w.tx().call_method(board, "evil_take_badge", args![]).put_last_instruction_output_on_workspace("badge")
        .call_method(owner.account, "deposit", args![Workspace("badge")]);
    w.run(b, &owner, &[]).expect_ok("(b) the owner takes the badge out");
    let _ = maker;
}

// ── Pause ────────────────────────────────────────────────────────────────────

#[test]
fn pause_cannot_be_abused_to_trap_escrow() {
    // ATTACK (owner): pause, change settings and leave it paused forever. Existing makers must still
    // get out, and existing orders must still fill.
    let mut w = world();
    let maker = w.party();
    let taker = w.party();
    let (cancel_s, cancel) = keypair();
    let a = w.place(&maker, SELL, N, P, Funds::Private, Funds::Private, cancel).order;
    let b = w.place(&maker, BUY, N, P, Funds::Private, Funds::Public, cancel).order;
    let board = w.board;
    let other = w.party();
    w.as_owner(w.tx()
        .call_method(board, "pause", args![])
        .call_method(board, "set_accounts", args![other.account, other.account])
        .call_method(board, "set_fee", args![10_000u16, 100u8]))
        .expect_ok("owner pauses and rewrites settings");
    w.cancel(&maker, &cancel_s, a).expect_ok("a maker still cancels");
    let n0 = w.balance(other.account, TARI_TOKEN);
    w.fill(&taker, b, Funds::Private, Funds::Private).0.expect_ok("an order still fills");
    assert_eq!(w.balance(other.account, TARI_TOKEN), n0, "the new settings took a share of an old order");
    // And a stranger cannot pause at all.
    let stranger = w.party();
    w.run(w.tx().call_method(board, "unpause", args![]), &stranger, &[]).expect_err("stranger unpause", "Access Denied");
}
