//! The Board: owner-only settings, pause stops NEW orders only, settings reach NEW orders only,
//! owner-only upgrade, and nothing the Board does reaches an existing order.

mod common;

use common::*;
use tari_template_lib::prelude::{Amount, TARI_TOKEN};
use tari_template_test_tooling::transaction::args;

const N: u64 = 100 * T;
const P: u64 = 50 * T;

#[test]
fn only_the_owner_can_change_settings_or_pause() {
    let mut w = world();
    let stranger = w.party();
    let board = w.board;
    let calls = [
        ("set_fee", w.tx().call_method(board, "set_fee", args![200u16, 50u8])),
        ("set_accounts", w.tx().call_method(board, "set_accounts", args![stranger.account, stranger.account])),
        ("set_min_order", w.tx().call_method(board, "set_min_order", args![Amount::from(T / 2)])),
        ("pause", w.tx().call_method(board, "pause", args![])),
        ("unpause", w.tx().call_method(board, "unpause", args![])),
    ];
    for (what, b) in calls {
        w.run(b, &stranger, &[]).expect_err(what, "Access Denied");
    }
    w.as_owner(w.tx().call_method(board, "set_fee", args![200u16, 50u8])).expect_ok("owner set_fee");
    w.as_owner(w.tx().call_method(board, "pause", args![])).expect_ok("owner pause");
}

#[test]
fn settings_are_range_checked() {
    let mut w = world();
    let board = w.board;
    w.as_owner(w.tx().call_method(board, "set_fee", args![10_001u16, 70u8])).expect_err("fee over 100%", "at most 100%");
    w.as_owner(w.tx().call_method(board, "set_fee", args![100u16, 101u8])).expect_err("share over 100", "percentage");
    w.as_owner(w.tx().call_method(board, "set_min_order", args![Amount::zero()])).expect_err("zero minimum", "positive");
}

#[test]
fn a_fee_above_10_percent_is_allowed() {
    let mut w = world();
    let board = w.board;
    w.as_owner(w.tx().call_method(board, "set_fee", args![2_000u16, 70u8])).expect_ok("a 20% fee");
    let maker = w.party();
    let taker = w.party();
    let (_, cancel) = keypair();
    let placed = w.place(&maker, SELL, N, P, Funds::Public, Funds::Public, cancel);
    placed.outcome.expect_ok("place at 20%");
    assert_eq!(w.order_fee(placed.order), 20 * T);
    assert_eq!(w.order_cuts(placed.order), (14 * T, 6 * T));
    let (c0, b0) = (w.caravel_tari(), w.burned());
    w.fill(&taker, placed.order, Funds::Public, Funds::Public).0.expect_ok("fill at 20%");
    assert_eq!(w.caravel_tari() - c0, Amount::from(14 * T));
    assert_eq!(w.burned() - b0, Amount::from(6 * T));
    w.as_owner(w.tx().call_method(board, "set_fee", args![10_000u16, 70u8])).expect_ok("exactly 100% is the bound");
}

#[test]
fn pause_stops_new_orders_only() {
    let mut w = world();
    let board = w.board;
    let maker = w.party();
    let taker = w.party();
    let (cancel_s, cancel) = keypair();
    let to_fill = w.place(&maker, SELL, N, P, Funds::Public, Funds::Public, cancel).order;
    let to_cancel = w.place(&maker, BUY, N, P, Funds::Public, Funds::Public, cancel).order;

    w.as_owner(w.tx().call_method(board, "pause", args![])).expect_ok("pause");
    let paused: bool = w.test.call_method(board, "is_paused", args![], vec![]);
    assert!(paused);
    for side in [SELL, BUY] {
        w.place(&maker, side, N, P, Funds::Public, Funds::Public, cancel).outcome.expect_err("place while paused", "paused");
    }
    w.fill(&taker, to_fill, Funds::Public, Funds::Public).0.expect_ok("fill while paused");
    w.cancel(&maker, &cancel_s, to_cancel).expect_ok("cancel while paused");

    w.as_owner(w.tx().call_method(board, "unpause", args![])).expect_ok("unpause");
    w.place(&maker, SELL, N, P, Funds::Public, Funds::Public, cancel).outcome.expect_ok("place after unpause");
}

#[test]
fn new_settings_reach_new_orders_only() {
    let mut w = world();
    let board = w.board;
    let maker = w.party();
    let taker = w.party();
    let new_fee_account = w.party();
    let (_, cancel) = keypair();
    let old_sell = w.place(&maker, SELL, N, P, Funds::Public, Funds::Public, cancel).order;
    let old_buy = w.place(&maker, BUY, N, P, Funds::Public, Funds::Public, cancel).order;

    // 2% fee, 50/50, all of it to a new account (as both fee account and "burn wallet"), minimum 10 tTARI.
    w.as_owner(w.tx()
        .call_method(board, "set_fee", args![200u16, 50u8])
        .call_method(board, "set_accounts", args![new_fee_account.account, new_fee_account.account])
        .call_method(board, "set_min_order", args![Amount::from(10 * T)]))
        .expect_ok("new settings");

    // The old orders fill on their ORIGINAL terms: 1%, 70/30, the original accounts.
    for order in [old_sell, old_buy] {
        let (c0, b0) = (w.caravel_tari(), w.burned());
        let n0 = w.balance(new_fee_account.account, TARI_TOKEN);
        w.fill(&taker, order, Funds::Public, Funds::Public).0.expect_ok("old order");
        assert_eq!(w.caravel_tari() - c0, Amount::from(700_000u64));
        assert_eq!(w.burned() - b0, Amount::from(300_000u64));
        assert_eq!(w.balance(new_fee_account.account, TARI_TOKEN), n0, "the new account got part of an old order's fee");
    }

    // A new order takes the new settings.
    w.place(&maker, SELL, 5 * T, P, Funds::Public, Funds::Public, cancel).outcome.expect_err("below the new minimum", "Below the minimum");
    let new = w.place(&maker, SELL, N, P, Funds::Public, Funds::Public, cancel).order;
    assert_eq!(w.order_fee(new), 2 * T);
    assert_eq!(w.order_cuts(new), (T, T));
    let (c0, b0) = (w.caravel_tari(), w.burned());
    let n0 = w.balance(new_fee_account.account, TARI_TOKEN);
    w.fill(&taker, new, Funds::Public, Funds::Public).0.expect_ok("new order");
    assert_eq!(w.balance(new_fee_account.account, TARI_TOKEN) - n0, Amount::from(2 * T));
    assert_eq!((w.caravel_tari(), w.burned()), (c0, b0));
}

#[test]
fn only_the_owner_can_upgrade_and_existing_orders_are_untouched() {
    let mut w = world();
    let board = w.board;
    let maker = w.party();
    let taker = w.party();
    let stranger = w.party();
    let (cancel_s, cancel) = keypair();
    let order = w.place(&maker, SELL, N, P, Funds::Public, Funds::Private, cancel);
    let other = w.place(&maker, BUY, N, P, Funds::Public, Funds::Public, cancel).order;
    let v2 = w.test.get_template_address("CaravelTradeBoardV2");

    w.run(w.tx().update_component_template(board, v2), &stranger, &[]).expect_err("stranger upgrade", "must be the owner");
    w.as_owner(w.tx().update_component_template(board, v2)).expect_ok("owner upgrade");
    let now = *w.test.read_only_state_store().get_component(board).unwrap().template_address();
    assert_eq!(now, v2, "the Board runs the new template");
    // `version` exists only in v2; the owner can call it, and the old state still reads.
    w.as_owner(w.tx().call_method(board, "version", args![])).expect_ok("a v2-only method");
    assert!(!w.test.extract_component_value::<bool>(board, "$.7"), "the Board's state survived the upgrade");

    // Nobody can switch an ORDER's code: it has no owner.
    w.as_owner(w.tx().update_component_template(order.order, v2)).expect_err("upgrading an order", "must be the owner");

    // The existing orders still fill and cancel exactly as before.
    let (o, _) = w.fill(&taker, order.order, Funds::Public, Funds::Public);
    o.expect_ok("fill after the Board upgrade");
    assert!(o.created(order.payout.as_ref().unwrap()));
    w.cancel(&maker, &cancel_s, other).expect_ok("cancel after the Board upgrade");

    // And the upgraded Board still places orders.
    w.place(&maker, SELL, N, P, Funds::Public, Funds::Public, cancel).outcome.expect_ok("place on the upgraded Board");
}
