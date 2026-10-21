//   TEST FIXTURE, never published: a HOSTILE Board upgrade (same state layout as templates/board),
//   for the red-team tests. It shows what a malicious or compromised Board owner could and could not do.

//   Caravel Trade Board: the settings and the entry point of the Caravel Trade order book.
//   Testnet only.
//
//   RULES
//     - Owned by the deployer's key. The owner rule is a constructor argument, so a future Board
//       can be owned by an m-of-n multisig rule instead.
//     - Settings: the fee (in basis points, 1% = 100), the Caravel/burn split (70/30), the Caravel
//       fee account, the Caravel Burn Wallet, the minimum order (1 tTARI) and the quote token (cUSD).
//     - Owner only: change the settings, pause and unpause NEW orders, and upgrade the Board (a
//       native owner action, so there is no method for it).
//     - place_sell / place_buy create an Order (a separate template) with a SNAPSHOT of the
//       settings and a proof of this Board's badge. After that the Board never touches the order:
//       new settings and pauses only ever affect orders placed afterwards, and fills and cancels
//       never pass through here.
//     - Each placed order emits an `OrderPlaced` event naming it.

use tari_template_lib::prelude::*;

/// 1 tTARI, in µT.
const ONE_TARI: u64 = 1_000_000;

#[template]
mod caravel_trade_board_hostile {
    use super::*;

    pub struct CaravelTradeBoardHostile {
        order_template: TemplateAddress,
        quote: ResourceAddress,
        /// The fee on the tTARI side of every trade, in basis points (100 = 1%).
        fee_bps: u16,
        /// Caravel's share of the fee, in percent; the Burn Wallet gets the rest.
        caravel_pct: u8,
        caravel_account: ComponentAddress,
        burn_wallet: ComponentAddress,
        /// The smallest tTARI side an order may have.
        min_order: Amount,
        /// While true, no new orders. Fills and cancels of existing orders are never affected.
        paused: bool,
        /// The Board's badge: one token, never minted again. Placing an order proves it.
        badge: Vault,
        /// The badge's address, kept in state so it is in scope when handed to the Order template.
        badge_resource: ResourceAddress,
    }

    impl CaravelTradeBoardHostile {
        /// A Board with the default settings: 1% fee, 70/30, minimum order 1 tTARI.
        pub fn new(
            owner: OwnerRule,
            order_template: TemplateAddress,
            quote: ResourceAddress,
            caravel_account: ComponentAddress,
            burn_wallet: ComponentAddress,
        ) -> Component<Self> {
            let badge = ResourceBuilder::public_fungible()
                .with_token_symbol("CTBOARD")
                .with_owner_rule(OwnerRule::None)
                .initial_supply(1u64);

            // Deny by default. The owner bypasses method rules, so the owner-only methods need no
            // rule of their own.
            let access_rules = ComponentAccessRules::new()
                .add_method_rule("place_sell_private", rule!(allow_all))
                .add_method_rule("place_sell_public", rule!(allow_all))
                .add_method_rule("place_buy_private", rule!(allow_all))
                .add_method_rule("place_buy_public", rule!(allow_all))
                .add_method_rule("is_paused", rule!(allow_all))
                .add_method_rule("version", rule!(allow_all))
                .default(rule!(deny_all));

            Component::new(Self {
                order_template,
                quote,
                fee_bps: 100,
                caravel_pct: 70,
                caravel_account,
                burn_wallet,
                min_order: Amount::from(ONE_TARI),
                paused: false,
                badge_resource: badge.resource_address(),
                badge: Vault::from_bucket(badge),
            })
            .with_owner_rule(owner)
            .with_access_rules(access_rules)
            .create()
        }

        // ── Placing orders (anyone) ──────────────────────────────────────────────

        /// Sell the escrowed tTARI for `p` cUSD, paid to the maker through `payout` (a pre-signed
        /// private statement, as CBOR). Returns the new order.
        pub fn place_sell_private(&self, escrow: Bucket, p: Amount, payout: Bytes, cancel_key: RistrettoPublicKeyBytes) -> ComponentAddress {
            let n = escrow.amount();
            self.place(SELL, escrow, n, p, payout, vec![], cancel_key)
        }

        /// Sell the escrowed tTARI for `p` cUSD, paid into the maker's `account`.
        pub fn place_sell_public(&self, escrow: Bucket, p: Amount, account: ComponentAddress, cancel_key: RistrettoPublicKeyBytes) -> ComponentAddress {
            let n = escrow.amount();
            self.place(SELL, escrow, n, p, Bytes::default(), vec![account], cancel_key)
        }

        /// Buy `n` tTARI with the escrowed cUSD; the maker receives N minus the fee through `payout`.
        pub fn place_buy_private(&self, escrow: Bucket, n: Amount, payout: Bytes, cancel_key: RistrettoPublicKeyBytes) -> ComponentAddress {
            let p = escrow.amount();
            self.place(BUY, escrow, n, p, payout, vec![], cancel_key)
        }

        /// Buy `n` tTARI with the escrowed cUSD; the maker receives N minus the fee in `account`.
        pub fn place_buy_public(&self, escrow: Bucket, n: Amount, account: ComponentAddress, cancel_key: RistrettoPublicKeyBytes) -> ComponentAddress {
            let p = escrow.amount();
            self.place(BUY, escrow, n, p, Bytes::default(), vec![account], cancel_key)
        }

        #[allow(clippy::too_many_arguments)]
        fn place(
            &self,
            side: u8,
            escrow: Bucket,
            n: Amount,
            p: Amount,
            payout_statement: Bytes,
            payout_account: Vec<ComponentAddress>,
            cancel_key: RistrettoPublicKeyBytes,
        ) -> ComponentAddress {
            assert!(!self.paused, "New orders are paused");
            assert!(n >= self.min_order, "Below the minimum order of {} µT", self.min_order);

            let order: ComponentAddress = TemplateManager::get(self.order_template).call("create", args![
                self.badge.create_proof(),
                self.badge_resource,
                CallerContext::current_component_address(),
                side,
                escrow,
                n,
                p,
                self.quote,
                self.fee_bps,
                self.caravel_pct,
                self.caravel_account,
                self.burn_wallet,
                payout_statement,
                payout_account,
                cancel_key
            ]);

            emit_event("OrderPlaced", metadata![
                "order" => order.to_string(),
                "side" => side.to_string(),
                "n" => n.to_string(),
                "p" => p.to_string(),
            ]);
            order
        }

        // ── Owner only ───────────────────────────────────────────────────────────

        /// The fee in basis points (at most 100%, 10,000 bps) and Caravel's share of it in percent.
        pub fn set_fee(&mut self, fee_bps: u16, caravel_pct: u8) {
            assert!(fee_bps <= 10_000, "The fee may be at most 100%");
            assert!(caravel_pct <= 100, "Caravel's share is a percentage");
            self.fee_bps = fee_bps;
            self.caravel_pct = caravel_pct;
        }

        pub fn set_accounts(&mut self, caravel_account: ComponentAddress, burn_wallet: ComponentAddress) {
            self.caravel_account = caravel_account;
            self.burn_wallet = burn_wallet;
        }

        pub fn set_min_order(&mut self, min_order: Amount) {
            assert!(min_order.is_positive(), "The minimum order must be positive");
            self.min_order = min_order;
        }

        pub fn pause(&mut self) {
            self.paused = true;
        }

        pub fn unpause(&mut self) {
            self.paused = false;
        }

        // ── Reads (anyone) ───────────────────────────────────────────────────────

        pub fn is_paused(&self) -> bool {
            self.paused
        }

        pub fn version(&self) -> u8 {
            2
        }

        // ── Hostile additions ────────────────────────────────────────────────────

        /// Cancel someone's existing order from inside the Board and keep the escrow.
        pub fn evil_cancel(&mut self, order: ComponentAddress) {
            let escrow: Bucket = ComponentManager::get(order).call("cancel", args![]);
            self.badge.deposit(escrow);
        }

        /// Take the Board's badge out (a compromised owner key could do this through an upgrade).
        pub fn evil_take_badge(&mut self) -> Bucket {
            self.badge.withdraw(Amount::from(1u64))
        }

        /// Place a "real" order with terms no honest Board would set: no fee, and a payout account
        /// of the owner's choosing. Uses the genuine Order template and the genuine badge.
        pub fn evil_place_zero_fee(&self, escrow: Bucket, p: Amount, account: ComponentAddress, cancel_key: RistrettoPublicKeyBytes) -> ComponentAddress {
            let n = escrow.amount();
            TemplateManager::get(self.order_template).call("create", args![
                self.badge.create_proof(),
                self.badge_resource,
                CallerContext::current_component_address(),
                SELL,
                escrow,
                n,
                p,
                self.quote,
                0u16,
                0u8,
                account,
                account,
                Bytes::default(),
                vec![account],
                cancel_key
            ])
        }
    }
}

const SELL: u8 = 0;
const BUY: u8 = 1;
