//! Orders: both sides, every public/private combination, fee math, amounts, once-only, cancel.

mod common;

use common::*;
use tari_template_lib::prelude::{Amount, TARI_TOKEN};
use tari_template_test_tooling::support::stealth::{NO_INPUTS, generate_transfer_data};
use tari_template_test_tooling::support::spec::OutputSpec;
use tari_template_test_tooling::transaction::args;

const N: u64 = 100 * T;
const P: u64 = 50 * T;

/// Every combination: side × maker funding × maker payout × taker payment × taker receipt (32).
#[test]
fn every_public_private_combination_on_both_sides() {
    let mut w = world();
    let mut count = 0;
    for side in [SELL, BUY] {
        for funding in BOTH {
            for paid in BOTH {
                for paying in BOTH {
                    for receiving in BOTH {
                        let label = format!("side={} maker funds={funding:?} maker paid={paid:?} taker pays={paying:?} taker receives={receiving:?}", if side == SELL { "SELL" } else { "BUY" });
                        let maker = w.party();
                        let taker = w.party();
                        let (_, cancel) = keypair();
                        let placed = w.place(&maker, side, N, P, funding, paid, cancel);
                        placed.outcome.expect_ok(&format!("place [{label}]"));
                        assert!(placed.outcome.has_event("OrderPlaced"), "no OrderPlaced event [{label}]");

                        let caravel0 = w.caravel_tari();
                        let burned0 = w.burned();
                        let maker_t0 = w.balance(maker.account, TARI_TOKEN);
                        let maker_c0 = w.balance(maker.account, w.cusd);
                        let taker_t0 = w.balance(taker.account, TARI_TOKEN);
                        let taker_c0 = w.balance(taker.account, w.cusd);

                        let (o, recv) = w.fill(&taker, placed.order, paying, receiving);
                        o.expect_ok(&format!("fill [{label}]"));
                        assert!(o.has_event("Filled"), "no Filled event [{label}]");
                        assert_eq!(w.status(placed.order), FILLED);

                        // The fee: 1 tTARI on 100, 0.7 to Caravel, 0.3 burned.
                        assert_eq!(w.caravel_tari() - caravel0, Amount::from(700_000u64), "Caravel's share [{label}]");
                        assert_eq!(w.burned() - burned0, Amount::from(300_000u64), "burned share [{label}]");

                        // The maker.
                        let maker_gets = if side == SELL { P } else { N - T };
                        match paid {
                            Funds::Private => assert!(o.created(placed.payout.as_ref().unwrap()), "maker's private output missing [{label}]"),
                            Funds::Public => {
                                let (res, before) = if side == SELL { (w.cusd, maker_c0) } else { (TARI_TOKEN, maker_t0) };
                                assert_eq!(w.balance(maker.account, res) - before, Amount::from(maker_gets), "maker's account [{label}]");
                            },
                        }

                        // The taker: received N - 1% tTARI on a SELL, P cUSD on a BUY.
                        let taker_gets = if side == SELL { N - T } else { P };
                        match receiving {
                            Funds::Private => assert!(o.created(recv.as_ref().unwrap()), "taker's private output missing [{label}]"),
                            Funds::Public => {
                                let (res, before) = if side == SELL { (TARI_TOKEN, taker_t0) } else { (w.cusd, taker_c0) };
                                // The taker pays in the other token, so this balance only moves by what they receive.
                                let expected = Amount::from(taker_gets);
                                let after = w.balance(taker.account, res);
                                assert_eq!(after - before, expected, "taker's account [{label}]");
                            },
                        }
                        count += 1;
                    }
                }
            }
        }
    }
    assert_eq!(count, 32);
}

#[test]
fn fee_math_rounds_down_and_the_split_remainder_is_burned() {
    let mut w = world();
    let cases: [(u64, u64, u64, u64); 4] = [
        // N, fee = floor(N/100), Caravel = floor(fee*70/100), burn = rest
        (1_000_000, 10_000, 7_000, 3_000),
        (1_234_567, 12_345, 8_641, 3_704),
        (1_000_099, 10_000, 7_000, 3_000),
        (99_999_999, 999_999, 699_999, 300_000),
    ];
    for side in [SELL, BUY] {
        for (n, fee, caravel, burn) in cases {
            let maker = w.party();
            let taker = w.party();
            let (_, cancel) = keypair();
            let placed = w.place(&maker, side, n, 3 * T, Funds::Public, Funds::Public, cancel);
            placed.outcome.expect_ok("place");
            assert_eq!(w.order_fee(placed.order), fee, "fee on {n}");
            assert_eq!(w.order_cuts(placed.order), (caravel, burn), "split on {n}");
            let (c0, b0) = (w.caravel_tari(), w.burned());
            let maker_t0 = w.balance(maker.account, TARI_TOKEN);
            let taker_t0 = w.balance(taker.account, TARI_TOKEN);
            w.fill(&taker, placed.order, Funds::Public, Funds::Public).0.expect_ok("fill");
            assert_eq!(w.caravel_tari() - c0, Amount::from(caravel));
            assert_eq!(w.burned() - b0, Amount::from(burn));
            if side == SELL {
                assert_eq!(w.balance(taker.account, TARI_TOKEN) - taker_t0, Amount::from(n - fee), "the tTARI receiver gets N - fee");
            } else {
                assert_eq!(w.balance(maker.account, TARI_TOKEN) - maker_t0, Amount::from(n - fee), "the tTARI receiver gets N - fee");
            }
        }
    }
}

#[test]
fn a_zero_share_is_never_deposited() {
    let mut w = world();
    // 100% to Caravel: the burn share is 0, and the Burn Wallet (which refuses a zero deposit) is
    // never called. Then 0% to Caravel: the whole fee is burned.
    for (pct, caravel, burn) in [(100u8, 10_000u64, 0u64), (0u8, 0, 10_000)] {
        let b = w.tx().call_method(w.board, "set_fee", args![100u16, pct]);
        w.as_owner(b).expect_ok("set_fee");
        let maker = w.party();
        let taker = w.party();
        let (_, cancel) = keypair();
        let placed = w.place(&maker, SELL, T, T, Funds::Public, Funds::Public, cancel);
        placed.outcome.expect_ok("place");
        let (c0, b0) = (w.caravel_tari(), w.burned());
        w.fill(&taker, placed.order, Funds::Public, Funds::Public).0.expect_ok("fill with a zero share");
        assert_eq!(w.caravel_tari() - c0, Amount::from(caravel));
        assert_eq!(w.burned() - b0, Amount::from(burn));
    }
}

#[test]
fn the_minimum_order_is_one_tari_on_both_sides() {
    let mut w = world();
    for side in [SELL, BUY] {
        let maker = w.party();
        let (_, cancel) = keypair();
        w.place(&maker, side, T - 1, T, Funds::Public, Funds::Public, cancel).outcome.expect_err("below the minimum", "Below the minimum");
        w.place(&maker, side, T, T, Funds::Public, Funds::Public, cancel).outcome.expect_ok("exactly the minimum");
    }
}

#[test]
fn wrong_amounts_tokens_and_terms_are_refused_and_the_order_stays_open() {
    let mut w = world();
    for side in [SELL, BUY] {
        let maker = w.party();
        let taker = w.party();
        let (_, cancel) = keypair();
        let placed = w.place(&maker, side, N, P, Funds::Public, Funds::Private, cancel);
        placed.outcome.expect_ok("place");
        let order = placed.order;
        let (due_res, due) = if side == SELL { (w.cusd, P) } else { (TARI_TOKEN, N) };
        let other_res = if side == SELL { TARI_TOKEN } else { w.cusd };
        for wrong in [due - 1, due + 1] {
            w.fill_with(&taker, order, side, N, P, due_res, wrong, Funds::Public, Funds::Public).0.expect_err("wrong amount", "Pay exactly");
        }
        w.fill_with(&taker, order, side, N, P, other_res, due, Funds::Public, Funds::Public).0.expect_err("wrong token", "Pay for a");
        w.fill_with(&taker, order, side, N + 1, P, due_res, due, Funds::Public, Funds::Public).0.expect_err("wrong expected N", "not what you expected");
        w.fill_with(&taker, order, side, N, P - 1, due_res, due, Funds::Public, Funds::Public).0.expect_err("wrong expected P", "not what you expected");
        assert_eq!(w.status(order), OPEN);
        w.fill(&taker, order, Funds::Public, Funds::Public).0.expect_ok("the right fill after all that");
    }
}

#[test]
fn an_order_fills_once_and_then_neither_fills_nor_cancels() {
    let mut w = world();
    for side in [SELL, BUY] {
        let maker = w.party();
        let (t1, t2) = (w.party(), w.party());
        let (cancel_s, cancel) = keypair();
        let placed = w.place(&maker, side, N, P, Funds::Public, Funds::Public, cancel);
        w.fill(&t1, placed.order, Funds::Public, Funds::Public).0.expect_ok("first fill");
        w.fill(&t2, placed.order, Funds::Public, Funds::Public).0.expect_err("second fill", "no longer open");
        w.cancel(&maker, &cancel_s, placed.order).expect_err("cancel after fill", "no longer open");
    }
}

#[test]
fn cancel_returns_all_the_escrow_with_no_fee_and_then_nothing_fills() {
    let mut w = world();
    for side in [SELL, BUY] {
        let maker = w.party();
        let taker = w.party();
        let (cancel_s, cancel) = keypair();
        let placed = w.place(&maker, side, N, P, Funds::Public, Funds::Public, cancel);
        let (res, escrow) = if side == SELL { (TARI_TOKEN, N) } else { (w.cusd, P) };
        let before = w.balance(maker.account, res);
        let (c0, b0) = (w.caravel_tari(), w.burned());
        let o = w.cancel(&maker, &cancel_s, placed.order);
        o.expect_ok("cancel");
        assert!(o.has_event("Cancelled"));
        assert_eq!(w.balance(maker.account, res) - before, Amount::from(escrow), "all the escrow comes back");
        assert_eq!((w.caravel_tari(), w.burned()), (c0, b0), "a cancel takes no fee");
        assert_eq!(w.status(placed.order), CANCELLED);
        w.fill(&taker, placed.order, Funds::Public, Funds::Public).0.expect_err("fill after cancel", "no longer open");
        w.cancel(&maker, &cancel_s, placed.order).expect_err("second cancel", "no longer open");
    }
}

#[test]
fn only_the_cancel_key_can_cancel() {
    let mut w = world();
    let maker = w.party();
    let (cancel_s, cancel) = keypair();
    let placed = w.place(&maker, SELL, N, P, Funds::Public, Funds::Public, cancel);
    // The maker's own account key is not the cancel key.
    let tx = w.tx().call_method(placed.order, "cancel", args![]).put_last_instruction_output_on_workspace("b")
        .call_method(maker.account, "deposit", args![Workspace("b")]).finish().seal(&maker.secret);
    w.exec(tx).expect_err("the maker's account key", "Access Denied");
    let stranger = w.party();
    let (wrong, _) = keypair();
    w.cancel(&stranger, &wrong, placed.order).expect_err("a stranger", "Access Denied");
    // The Board's owner cannot either.
    let owner = Party { account: w.owner.account, secret: w.owner.secret.clone(), pk: w.owner.pk };
    let tx = w.tx().call_method(placed.order, "cancel", args![]).put_last_instruction_output_on_workspace("b")
        .call_method(owner.account, "deposit", args![Workspace("b")]).finish().seal(&owner.secret);
    w.exec(tx).expect_err("the Board owner", "Access Denied");
    w.cancel(&stranger, &cancel_s, placed.order).expect_ok("the cancel key (whoever's transaction carries it)");
}

#[test]
fn bad_payout_statements_are_refused_when_placing() {
    let mut w = world();
    let maker = w.party();
    let (_, cancel) = keypair();
    let place_with = |w: &mut World, bytes| {
        let b = w.tx()
            .call_method(maker.account, "withdraw", args![TARI_TOKEN, Amount::from(N)]).put_last_instruction_output_on_workspace("e")
            .call_method(w.board, "place_sell_private", args![Workspace("e"), Amount::from(P), bytes, cancel]);
        w.run(b, &maker, &[])
    };
    let (_, wrong_amount) = payout(P + 1);
    place_with(&mut w, wrong_amount).expect_err("wrong amount", "exactly what the maker is owed");
    let two = generate_transfer_data(NO_INPUTS, Amount::from(P), [OutputSpec::key_path_from_mask(P / 2), OutputSpec::key_path_from_mask(P / 2)], Amount::zero());
    place_with(&mut w, encode(&two.statement)).expect_err("two outputs", "exactly one private output");
    let revealing = generate_transfer_data(NO_INPUTS, Amount::from(P), [OutputSpec::key_path_from_mask(P - 1)], Amount::from(1u64));
    place_with(&mut w, encode(&revealing.statement)).expect_err("a revealed output", "reveal nothing");
    place_with(&mut w, tari_template_lib::types::bytes::Bytes::from_vec(vec![1, 2, 3])).expect_err("garbage", "unreadable");
    place_with(&mut w, tari_template_lib::types::bytes::Bytes::default()).expect_err("empty", "exactly one way");
    // A BUY maker's payout must be N - fee, not N.
    let (_, n_not_net) = payout(N);
    let b = w.tx()
        .call_method(w.faucet, "claim", args![maker.pk]).put_last_instruction_output_on_workspace("c")
        .call_method(maker.account, "deposit", args![Workspace("c")])
        .call_method(maker.account, "withdraw", args![w.cusd, Amount::from(P)]).put_last_instruction_output_on_workspace("e")
        .call_method(w.board, "place_buy_private", args![Workspace("e"), Amount::from(N), n_not_net, cancel]);
    w.run(b, &maker, &[]).expect_err("buy payout of N instead of N - fee", "exactly what the maker is owed");
}

#[test]
fn an_order_made_outside_the_board_cannot_name_the_boards_badge() {
    let mut w = world();
    let forger = w.party();
    w.claim_to_account(&forger).expect_ok("claim");
    let order_template = w.test.get_template_address("CaravelTradeOrder");
    let real_badge: tari_template_lib::prelude::ResourceAddress = w.test.extract_component_value(w.board, "$.9");
    let (_, bytes) = payout(P);
    let (_, cancel) = keypair();
    // A proof of cUSD (anything the forger holds), naming the real badge.
    let b = w.tx()
        .create_proof(forger.account, w.cusd).put_last_instruction_output_on_workspace("proof")
        .call_method(forger.account, "withdraw", args![TARI_TOKEN, Amount::from(N)]).put_last_instruction_output_on_workspace("e")
        .call_function(order_template, "create", args![
            Workspace("proof"), real_badge, w.board, SELL, Workspace("e"), Amount::from(N), Amount::from(P), w.cusd,
            0u16, 0u8, forger.account, forger.account, bytes, Vec::<tari_template_lib::prelude::ComponentAddress>::new(), cancel
        ]);
    w.run(b, &forger, &[]).expect_err("forged order", "Proof of resource did not match");
}
