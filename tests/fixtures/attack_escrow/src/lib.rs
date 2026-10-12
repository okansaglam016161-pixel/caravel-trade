//   TEST FIXTURE, never published: red-team attackers for the escrow and fills.
//
//   A HostileAccount is what a malicious maker can name as their public payout account. Its
//   `deposit` runs inside the taker's fill, so it is the one place a maker's code executes during a
//   trade. `mode` picks what it does there:
//     0 accept and keep          1 re-enter `fill` on the order with what it was just paid
//     2 re-enter `cancel`        3 refuse (panic)

use tari_template_lib::prelude::*;

#[template]
mod caravel_attack_escrow {
    use super::*;

    pub struct HostileAccount {
        mode: u8,
        /// The order to attack; empty until set.
        target: Vec<ComponentAddress>,
        expected_n: Amount,
        expected_p: Amount,
        vaults: Vec<Vault>,
    }

    impl HostileAccount {
        pub fn new(mode: u8) -> Component<Self> {
            Component::new(Self { mode, target: vec![], expected_n: Amount::zero(), expected_p: Amount::zero(), vaults: vec![] })
                .with_access_rules(AccessRules::allow_all())
                .with_owner_rule(OwnerRule::None)
                .create()
        }

        pub fn set_target(&mut self, order: ComponentAddress, n: Amount, p: Amount) {
            self.target = vec![order];
            self.expected_n = n;
            self.expected_p = p;
        }

        pub fn deposit(&mut self, bucket: Bucket) {
            match self.mode {
                1 => {
                    let back: Bucket = ComponentManager::get(self.target[0]).call("fill", args![bucket, self.expected_n, self.expected_p]);
                    self.keep(back);
                },
                2 => {
                    let back: Bucket = ComponentManager::get(self.target[0]).call("cancel", args![]);
                    self.keep(back);
                    self.keep(bucket);
                },
                3 => panic!("this account refuses deposits"),
                _ => self.keep(bucket),
            }
        }

        fn keep(&mut self, bucket: Bucket) {
            let res = bucket.resource_address();
            if let Some(v) = self.vaults.iter().find(|v| v.resource_address() == res) {
                v.deposit(bucket);
            } else {
                self.vaults.push(Vault::from_bucket(bucket));
            }
        }

        /// A stealth token dressed as cUSD: same symbol, same name.
        pub fn fake_cusd(amount: Amount) -> Bucket {
            ResourceBuilder::stealth()
                .with_token_symbol("cUSD")
                .add_metadata("name", "Caravel Test Dollar")
                .initial_supply(amount)
        }

        pub fn balance(&self, resource: ResourceAddress) -> Amount {
            self.vaults.iter().find(|v| v.resource_address() == resource).map(|v| v.balance()).unwrap_or_default()
        }
    }
}
