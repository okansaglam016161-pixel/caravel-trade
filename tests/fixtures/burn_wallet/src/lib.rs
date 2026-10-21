//   Caravel Burn Wallet — a deposit-only TARI wallet with no owner.
//
//   RULES
//     - Anyone can deposit TARI. Only TARI: any other resource is refused.
//     - Nothing can ever leave. The template has no method that withdraws, recalls, burns or
//       otherwise moves TARI out, and no method that changes access rules or the owner.
//     - No owner. The component is created with `OwnerRule::None`, which the engine evaluates as
//       "no caller is the owner" — so nobody, the deployer included, can bypass the method rules
//       below or change them later. The default owner rule would be the deployer's key; it is
//       replaced explicitly.
//     - Public: `balance()` and `total_deposited()` are open reads, and every deposit emits a
//       `Deposit` event with its amount.
//
//   Because nothing leaves, `balance()` and `total_deposited()` are always equal. The running total
//   is kept anyway, so the record of what was deposited does not depend on reading the vault.

use tari_template_lib::prelude::*;

#[template]
mod caravel_burn_wallet {
    use super::*;

    pub struct CaravelBurnWallet {
        /// The TARI held, forever.
        vault: Vault,
        /// Every µTARI ever deposited.
        total_deposited: Amount,
    }

    impl CaravelBurnWallet {
        /// Create an empty burn wallet with no owner.
        pub fn new() -> Component<Self> {
            // Deny by default. With no owner, nobody bypasses these rules: the three listed
            // methods are the whole public surface.
            let access_rules = ComponentAccessRules::new()
                .add_method_rule("deposit", rule!(allow_all))
                .add_method_rule("balance", rule!(allow_all))
                .add_method_rule("total_deposited", rule!(allow_all))
                .default(rule!(deny_all));

            Component::new(Self {
                vault: Vault::new_empty(TARI_TOKEN),
                total_deposited: Amount::zero(),
            })
            .with_owner_rule(OwnerRule::None)
            .with_access_rules(access_rules)
            .create()
        }

        /// Lock TARI in the burn wallet, forever. Anyone may deposit.
        pub fn deposit(&mut self, bucket: Bucket) {
            assert!(
                bucket.resource_address() == TARI_TOKEN,
                "Only TARI can be deposited into the burn wallet"
            );
            let amount = bucket.amount();
            assert!(amount.is_positive(), "Deposit must be greater than zero");

            self.vault.deposit(bucket);
            self.total_deposited = self
                .total_deposited
                .checked_add(amount)
                .expect("total_deposited overflowed");

            emit_event("Deposit", metadata!["amount" => amount.to_string()]);
        }

        /// The TARI held, in µTARI.
        pub fn balance(&self) -> Amount {
            self.vault.balance()
        }

        /// Every µTARI ever deposited.
        pub fn total_deposited(&self) -> Amount {
            self.total_deposited
        }
    }
}
