//! RED TEAM: the escrow and fills. Stealing or double-claiming funds, wrong payouts, look-alike
//! tokens, a maker's own code running inside a fill, statement replay and tampering, same-transaction
//! tricks, overflow. Each test asserts the attack does no harm (or harms only the attacker).

mod common;

use common::*;
use tari_template_lib::prelude::{Amount, ComponentAddress, OwnerRule, ResourceAddress, TARI_TOKEN};
use tari_template_lib::types::bytes::Bytes;
use tari_template_lib::types::stealth::SpendAuthorization;
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

fn hostile(w: &mut World, mode: u8) -> ComponentAddress {
    w.test.call_function("HostileAccount", "new", args![mode], vec![])
}

/// A SELL whose maker is paid into `account` (any component the maker names).
fn sell_paying_into(w: &mut World, maker: &Party, account: ComponentAddress, cancel: tari_template_lib::types::crypto::RistrettoPublicKeyBytes) -> ComponentAddress {
    let b = w.tx()
        .call_method(maker.account, "withdraw", args![TARI_TOKEN, Amount::from(N)]).put_last_instruction_output_on_workspace("e")
        .call_method(w.board, "place_sell_public", args![Workspace("e"), Amount::from(P), account, cancel]);
    let o = w.run(b, maker, &[]);
    o.expect_ok("place");
    w.order_in(&o)
}

/// A SELL whose maker is paid by the given statement bytes.
fn sell_with_statement(w: &mut World, maker: &Party, bytes: Bytes, cancel: tari_template_lib::types::crypto::RistrettoPublicKeyBytes) -> ComponentAddress {
    let b = w.tx()
        .call_method(maker.account, "withdraw", args![TARI_TOKEN, Amount::from(N)]).put_last_instruction_output_on_workspace("e")
        .call_method(w.board, "place_sell_private", args![Workspace("e"), Amount::from(P), bytes, cancel]);
    let o = w.run(b, maker, &[]);
    o.expect_ok("place");
    w.order_in(&o)
}

// ── Look-alike tokens ────────────────────────────────────────────────────────

#[test]
fn a_look_alike_cusd_cannot_pay_for_an_order() {
    // ATTACK: pay a SELL with a stealth token that has cUSD's symbol and name.
    let mut w = world();
    let maker = w.party();
    let taker = w.party();
    let (_, cancel) = keypair();
    let order = w.place(&maker, SELL, N, P, Funds::Public, Funds::Public, cancel).order;
    let tpl = w.test.get_template_address("HostileAccount");
    let b = w.tx()
        .call_function(tpl, "fake_cusd", args![Amount::from(P)]).put_last_instruction_output_on_workspace("fake")
        .call_method(order, "fill", args![Workspace("fake"), Amount::from(N), Amount::from(P)]).put_last_instruction_output_on_workspace("got")
        .call_method(taker.account, "deposit", args![Workspace("got")]);
    w.run(b, &taker, &[]).expect_err("fake cUSD", "Pay for a sell order in cUSD");
    assert_eq!(w.status(order), OPEN);
}

// ── The maker's own code inside a fill ───────────────────────────────────────

#[test]
fn a_hostile_payout_account_cannot_reenter_fill_or_cancel() {
    // ATTACK: the maker names a component of their own as the payout account. Its `deposit` runs
    // inside the taker's fill: mode 1 re-enters `fill` with the cUSD it was just paid, mode 2 calls
    // `cancel` to pull the escrow back mid-fill.
    let mut w = world();
    for mode in [1u8, 2u8] {
        let maker = w.party();
        let taker = w.party();
        let (cancel_s, cancel) = keypair();
        let acct = hostile(&mut w, mode);
        let order = sell_paying_into(&mut w, &maker, acct, cancel);
        w.test.call_method::<()>(acct, "set_target", args![order, Amount::from(N), Amount::from(P)], vec![]);
        let t0 = w.balance(taker.account, TARI_TOKEN);
        let (o, _) = w.fill(&taker, order, Funds::Public, Funds::Public);
        println!("hostile payout account, mode {mode}: {}", o.error);
        assert!(!o.ok(), "mode {mode}: the re-entrant fill went through");
        // Nothing moved: the whole transaction was refused, the order is still open.
        assert_eq!(w.status(order), OPEN);
        assert_eq!(w.balance(taker.account, TARI_TOKEN), t0);
        // GRIEF ONLY: the order can never be filled, but the maker can still take the escrow back.
        w.cancel(&maker, &cancel_s, order).expect_ok("the maker still cancels");
    }
    // Control: a payout component that simply accepts (mode 0) is paid exactly P cUSD.
    let maker = w.party();
    let taker = w.party();
    let (_, cancel) = keypair();
    let acct = hostile(&mut w, 0);
    let order = sell_paying_into(&mut w, &maker, acct, cancel);
    w.fill(&taker, order, Funds::Public, Funds::Public).0.expect_ok("control fill");
    let got: Amount = w.test.call_method(acct, "balance", args![w.cusd], vec![]);
    assert_eq!(got, Amount::from(P));
}

// ── Statement replay and tampering ───────────────────────────────────────────

#[test]
fn a_reused_payout_statement_pays_once_and_the_second_order_stays_cancellable() {
    // ATTACK (maker): one statement in two orders, hoping to be paid twice.
    let mut w = world();
    let maker = w.party();
    let (t1, t2) = (w.party(), w.party());
    let (cancel_s, cancel) = keypair();
    let (stmt, bytes) = payout(P);
    let first = sell_with_statement(&mut w, &maker, bytes.clone(), cancel);
    let second = sell_with_statement(&mut w, &maker, bytes, cancel);
    let (o, _) = w.fill(&t1, first, Funds::Public, Funds::Public);
    o.expect_ok("first fill");
    assert!(o.created(&stmt));
    let (o, _) = w.fill(&t2, second, Funds::Public, Funds::Public);
    println!("replayed statement: {}", o.error);
    assert!(!o.ok(), "the same output was created twice");
    assert_eq!(w.status(second), OPEN);
    w.cancel(&maker, &cancel_s, second).expect_ok("the second order is still cancellable");
}

#[test]
fn copying_someone_elses_payout_only_pays_them() {
    // ATTACK: copy a victim's payout statement (it is public in their order's state) into the
    // attacker's own order. The attacker's taker then pays the VICTIM, and the victim's own order can
    // no longer be filled. The victim is paid without giving up anything and cancels for the escrow.
    let mut w = world();
    let victim = w.party();
    let attacker = w.party();
    let taker = w.party();
    let (victim_cancel_s, victim_cancel) = keypair();
    let (_, attacker_cancel) = keypair();
    let placed = w.place(&victim, SELL, N, P, Funds::Public, Funds::Private, victim_cancel);
    let copied: Bytes = w.test.extract_component_value(placed.order, "$.12");
    let a_order = sell_with_statement(&mut w, &attacker, copied, attacker_cancel);
    let a0 = w.balance(attacker.account, TARI_TOKEN);
    let (o, _) = w.fill(&taker, a_order, Funds::Public, Funds::Public);
    o.expect_ok("the attacker's order fills");
    assert!(o.created(placed.payout.as_ref().unwrap()), "the victim's output was created: the victim was paid");
    assert_eq!(w.balance(attacker.account, TARI_TOKEN), a0, "the attacker got nothing back");
    let (o, _) = w.fill(&taker, placed.order, Funds::Public, Funds::Public);
    assert!(!o.ok(), "the victim's order should now be unfillable");
    let v0 = w.balance(victim.account, TARI_TOKEN);
    w.cancel(&victim, &victim_cancel_s, placed.order).expect_ok("the victim cancels");
    assert_eq!(w.balance(victim.account, TARI_TOKEN) - v0, Amount::from(N), "the victim keeps the escrow too");
}

#[test]
fn a_tampered_statement_cannot_redirect_a_payout_and_the_order_stays_cancellable() {
    // ATTACK: re-target a payout statement's output to another key. It passes the placement checks
    // (they check its shape) but the balance proof binds the output, so no fill can run it.
    let mut w = world();
    let maker = w.party();
    let taker = w.party();
    let (cancel_s, cancel) = keypair();
    let (mut stmt, _) = payout(P);
    let (_, thief) = keypair();
    stmt.outputs_statement.outputs[0].auth = SpendAuthorization::Key(thief);
    let order = sell_with_statement(&mut w, &maker, encode(&stmt), cancel);
    let (o, _) = w.fill(&taker, order, Funds::Public, Funds::Public);
    println!("tampered statement at fill: {}", o.error);
    assert!(!o.ok(), "a re-targeted payout executed");
    assert_eq!(w.status(order), OPEN);
    w.cancel(&maker, &cancel_s, order).expect_ok("cancel");
}

// ── Same-transaction tricks ──────────────────────────────────────────────────

#[test]
fn two_fills_in_one_transaction_are_refused_and_nothing_moves() {
    let mut w = world();
    let maker = w.party();
    let taker = w.party();
    let (_, cancel) = keypair();
    let order = w.place(&maker, SELL, N, P, Funds::Public, Funds::Public, cancel).order;
    w.claim_to_account(&taker).expect_ok("claim");
    let (t0, c0) = (w.balance(taker.account, TARI_TOKEN), w.balance(taker.account, w.cusd));
    let b = w.tx()
        .call_method(taker.account, "withdraw", args![w.cusd, Amount::from(P)]).put_last_instruction_output_on_workspace("p1")
        .call_method(order, "fill", args![Workspace("p1"), Amount::from(N), Amount::from(P)]).put_last_instruction_output_on_workspace("g1")
        .call_method(taker.account, "withdraw", args![w.cusd, Amount::from(P)]).put_last_instruction_output_on_workspace("p2")
        .call_method(order, "fill", args![Workspace("p2"), Amount::from(N), Amount::from(P)]).put_last_instruction_output_on_workspace("g2")
        .call_method(taker.account, "deposit", args![Workspace("g1")])
        .call_method(taker.account, "deposit", args![Workspace("g2")]);
    w.run(b, &taker, &[]).expect_err("double fill", "no longer open");
    assert_eq!((w.balance(taker.account, TARI_TOKEN), w.balance(taker.account, w.cusd)), (t0, c0));
    assert_eq!(w.status(order), OPEN);
}

#[test]
fn cancel_and_fill_in_one_transaction_are_refused_in_either_order() {
    let mut w = world();
    let maker = w.party();
    let (cancel_s, cancel) = keypair();
    let order = w.place(&maker, SELL, N, P, Funds::Public, Funds::Public, cancel).order;
    w.claim_to_account(&maker).expect_ok("claim");
    let cancel_then_fill = w.tx()
        .call_method(order, "cancel", args![]).put_last_instruction_output_on_workspace("back")
        .call_method(maker.account, "deposit", args![Workspace("back")])
        .call_method(maker.account, "withdraw", args![w.cusd, Amount::from(P)]).put_last_instruction_output_on_workspace("p")
        .call_method(order, "fill", args![Workspace("p"), Amount::from(N), Amount::from(P)]).put_last_instruction_output_on_workspace("g")
        .call_method(maker.account, "deposit", args![Workspace("g")]);
    let tx = cancel_then_fill.finish().add_signer(&maker.pk, &cancel_s).seal(&maker.secret);
    w.exec(tx).expect_err("cancel then fill", "no longer open");
    let fill_then_cancel = w.tx()
        .call_method(maker.account, "withdraw", args![w.cusd, Amount::from(P)]).put_last_instruction_output_on_workspace("p")
        .call_method(order, "fill", args![Workspace("p"), Amount::from(N), Amount::from(P)]).put_last_instruction_output_on_workspace("g")
        .call_method(maker.account, "deposit", args![Workspace("g")])
        .call_method(order, "cancel", args![]).put_last_instruction_output_on_workspace("back")
        .call_method(maker.account, "deposit", args![Workspace("back")]);
    let tx = fill_then_cancel.finish().add_signer(&maker.pk, &cancel_s).seal(&maker.secret);
    w.exec(tx).expect_err("fill then cancel", "no longer open");
    assert_eq!(w.status(order), OPEN);
}

// ── Overflow and extremes ────────────────────────────────────────────────────

#[test]
fn absurd_amounts_overflow_into_a_refusal_or_an_order_only_its_maker_can_close() {
    let mut w = world();
    let maker = w.party();
    let (cancel_s, cancel) = keypair();
    w.claim_to_account(&maker).expect_ok("claim");
    let buy = |w: &mut World, n: Amount| {
        let b = w.tx()
            .call_method(maker.account, "withdraw", args![w.cusd, Amount::from(P)]).put_last_instruction_output_on_workspace("e")
            .call_method(w.board, "place_buy_public", args![Workspace("e"), n, maker.account, cancel]);
        w.run(b, &maker, &[])
    };
    // N = u128::MAX: the fee multiplication overflows and panics; nothing is placed.
    let o = buy(&mut w, Amount::MAX);
    println!("BUY with N = Amount::MAX: {}", o.error);
    assert!(!o.ok());
    // N = 10^30: placed, but nobody can ever pay it. Only the maker is affected, and they can cancel.
    let o = buy(&mut w, Amount::from(10u64).checked_pow(30).unwrap());
    o.expect_ok("an unpayable BUY is placed");
    let order = w.order_in(&o);
    w.cancel(&maker, &cancel_s, order).expect_ok("its maker cancels");
    // P = 1 base unit: a legal, tiny price; the fee is on the tTARI side and is still 1%.
    let tiny = w.place(&maker, SELL, N, 1, Funds::Public, Funds::Public, cancel).order;
    assert_eq!(w.order_fee(tiny), T);
}
