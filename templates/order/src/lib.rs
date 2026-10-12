//   Caravel Trade Order: one whole order, placed by the Caravel Trade Board. Testnet only.
//
//   RULES
//     - SELL: the maker escrows N tTARI and wants P cUSD.
//       BUY:  the maker escrows P cUSD and wants N tTARI.
//     - Terms are frozen when the order is placed, including a snapshot of the Board's fee
//       settings. The order never calls or reads the Board again: nothing the Board does later, a
//       pause, new settings or an upgrade, reaches an order that already exists.
//     - No owner. Nobody can change this component's rules or code.
//     - fill(): whole order only. The taker states the N and P they expect, and pays exactly the
//       side they owe. The fee is on the tTARI side: fee = floor(N × fee_bps / 10,000), which at the
//       Board's 1% is floor(N / 100). Caravel gets floor(fee × caravel_pct / 100), the Caravel Burn
//       Wallet the remainder, and a zero share is never deposited. Whoever receives tTARI gets
//       N minus the fee.
//     - The maker is paid by their own pre-signed private statement (stored as CBOR bytes, so the
//       indexer cache never sees a null) or by a deposit into a public account they named. The
//       taker's proceeds come back as a bucket for the taker's transaction to place.
//     - cancel(): only with the order's cancel key; returns all the escrow; no fee.
//     - An order can be filled or cancelled once. After that every fill and cancel is refused.
//     - No `Option` anywhere in state: an absent payout is an empty `Bytes` or an empty list.

use tari_template_lib::prelude::*;

pub const SELL: u8 = 0;
pub const BUY: u8 = 1;

pub const OPEN: u8 = 0;
pub const FILLED: u8 = 1;
pub const CANCELLED: u8 = 2;

#[template]
mod caravel_trade_order {
    use super::*;

    pub struct CaravelTradeOrder {
        /// SELL (0) or BUY (1).
        side: u8,
        /// OPEN (0), FILLED (1) or CANCELLED (2).
        status: u8,
        /// The Board that placed this order, and its badge. Only a holder of that badge can place an
        /// order naming it, so these two fields tell a real order from a look-alike.
        board: ComponentAddress,
        board_badge: ResourceAddress,
        /// The quote token (cUSD).
        quote: ResourceAddress,
        /// The tTARI side of the trade.
        n: Amount,
        /// The cUSD side of the trade.
        p: Amount,
        /// The fee and its split, computed once from the Board's settings when the order was placed.
        fee: Amount,
        caravel_cut: Amount,
        burn_cut: Amount,
        caravel_account: ComponentAddress,
        burn_wallet: ComponentAddress,
        /// The maker's pre-signed private payout, as CBOR. Empty when the maker is paid publicly.
        payout_statement: Bytes,
        /// The maker's public account. Empty when the maker is paid privately; never more than one.
        payout_account: Vec<ComponentAddress>,
        /// N tTARI for a SELL, P cUSD for a BUY.
        escrow: Vault,
    }

    impl CaravelTradeOrder {
        /// Called by the Board only: `board_proof` must prove the Board's badge.
        #[allow(clippy::too_many_arguments)]
        pub fn create(
            board_proof: Proof,
            board_badge: ResourceAddress,
            board: ComponentAddress,
            side: u8,
            escrow: Bucket,
            n: Amount,
            p: Amount,
            quote: ResourceAddress,
            fee_bps: u16,
            caravel_pct: u8,
            caravel_account: ComponentAddress,
            burn_wallet: ComponentAddress,
            payout_statement: Bytes,
            payout_account: Vec<ComponentAddress>,
            cancel_key: RistrettoPublicKeyBytes,
        ) -> Component<Self> {
            board_proof.assert_resource(board_badge);
            board_proof.drop();

            assert!(side == SELL || side == BUY, "The side must be SELL or BUY");
            assert!(n.is_positive() && p.is_positive(), "Both amounts must be positive");
            assert!(fee_bps <= 1_000 && caravel_pct <= 100, "Fee settings out of range");

            let fee = n * Amount::from(fee_bps) / Amount::from(10_000u64);
            let caravel_cut = fee * Amount::from(caravel_pct) / Amount::from(100u64);
            let burn_cut = fee - caravel_cut;

            let (escrow_token, escrow_amount) = if side == SELL { (TARI_TOKEN, n) } else { (quote, p) };
            assert!(escrow.resource_address() == escrow_token, "The escrow is in the wrong token");
            assert!(escrow.amount() == escrow_amount, "The escrow must be exactly the order amount");

            // The maker is owed P cUSD for a SELL, N minus the fee in tTARI for a BUY.
            let owed = if side == SELL { p } else { n - fee };
            match (payout_statement.is_empty(), payout_account.len()) {
                (false, 0) => check_payout(&payout_statement, owed),
                (true, 1) => {},
                _ => panic!("Name exactly one way to pay the maker: a private statement or one account"),
            }

            let access_rules = ComponentAccessRules::new()
                .add_method_rule("fill", rule!(allow_all))
                .add_method_rule("cancel", rule!(public_key(cancel_key)))
                .default(rule!(deny_all));

            Component::new(Self {
                side,
                status: OPEN,
                board,
                board_badge,
                quote,
                n,
                p,
                fee,
                caravel_cut,
                burn_cut,
                caravel_account,
                burn_wallet,
                payout_statement,
                payout_account,
                escrow: Vault::from_bucket(escrow),
            })
            .with_owner_rule(OwnerRule::None)
            .with_access_rules(access_rules)
            .create()
        }

        /// Take the whole order. `expected_n` and `expected_p` must match its terms; `payment` is
        /// exactly P cUSD for a SELL or N tTARI for a BUY. Returns what the taker receives.
        pub fn fill(&mut self, payment: Bucket, expected_n: Amount, expected_p: Amount) -> Bucket {
            assert!(self.status == OPEN, "This order is no longer open");
            assert!(expected_n == self.n && expected_p == self.p, "The order's terms are not what you expected");
            self.status = FILLED;

            let to_taker = if self.side == SELL {
                assert!(payment.resource_address() == self.quote, "Pay for a sell order in cUSD");
                assert!(payment.amount() == self.p, "Pay exactly the order's cUSD amount");
                self.pay_maker(payment);
                let mut tari = self.escrow.withdraw(self.n);
                self.pay_fee(&mut tari);
                tari
            } else {
                assert!(payment.resource_address() == TARI_TOKEN, "Pay for a buy order in tTARI");
                assert!(payment.amount() == self.n, "Pay exactly the order's tTARI amount");
                let mut tari = payment;
                self.pay_fee(&mut tari);
                self.pay_maker(tari);
                self.escrow.withdraw(self.p)
            };

            emit_event("Filled", metadata![
                "side" => self.side.to_string(),
                "n" => self.n.to_string(),
                "p" => self.p.to_string(),
                "fee" => self.fee.to_string(),
            ]);
            to_taker
        }

        /// Return all the escrow to the maker. Signed by the order's cancel key only; no fee.
        pub fn cancel(&mut self) -> Bucket {
            assert!(self.status == OPEN, "This order is no longer open");
            self.status = CANCELLED;
            emit_event("Cancelled", metadata!["side" => self.side.to_string()]);
            let all = self.escrow.balance();
            self.escrow.withdraw(all)
        }

        /// The fee's two shares out of `tari`; a zero share is never deposited.
        fn pay_fee(&self, tari: &mut Bucket) {
            if self.caravel_cut.is_positive() {
                let cut = tari.take(self.caravel_cut);
                ComponentManager::get(self.caravel_account).invoke("deposit", args![cut]);
            }
            if self.burn_cut.is_positive() {
                let cut = tari.take(self.burn_cut);
                ComponentManager::get(self.burn_wallet).invoke("deposit", args![cut]);
            }
        }

        /// The whole of `funds` to the maker: through their statement, or into their account.
        fn pay_maker(&self, funds: Bucket) {
            if self.payout_statement.is_empty() {
                ComponentManager::get(self.payout_account[0]).invoke("deposit", args![funds]);
            } else {
                let statement: StealthTransferStatement =
                    tari_bor::decode(&self.payout_statement).expect("The stored payout statement is unreadable");
                let left = funds.stealth_transfer(statement);
                assert!(left.amount().is_zero(), "The payout must take the whole amount");
                left.drop_empty();
            }
        }
    }
}

/// A maker's payout statement must turn exactly `owed` revealed funds into one private output and
/// nothing else. Its balance proof is checked when it runs, at fill; a taker's dry run sees that.
fn check_payout(bytes: &Bytes, owed: Amount) {
    let s: StealthTransferStatement = tari_bor::decode(bytes).expect("The payout statement is unreadable");
    assert!(s.inputs_statement.inputs.is_empty(), "The payout must spend no private inputs");
    assert!(s.inputs_statement.revealed_amount == owed, "The payout must spend exactly what the maker is owed");
    assert!(s.outputs_statement.revealed_output.is_none(), "The payout must reveal nothing");
    assert!(s.outputs_statement.outputs.len() == 1, "The payout must create exactly one private output");
    assert!(s.balance_proof.is_some(), "The payout must carry a balance proof");
    assert!(s.covenant_claims.is_empty(), "The payout must carry no covenant claims");
}
