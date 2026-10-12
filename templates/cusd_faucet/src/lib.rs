//   Caravel Test Dollar (cUSD) and its faucet. Testnet only.
//
//   RULES
//     - cUSD is a STEALTH resource with a fixed supply of 1,000,000,000 cUSD, created once, here,
//       straight into the faucet. Minting, burning, recalling and freezing are denied and locked,
//       and the resource has no owner: nobody, the deployer included, can ever create, destroy,
//       take back or freeze a cUSD.
//     - The faucet has no owner either. Anyone may claim; nobody can withdraw or change anything.
//     - ONE claim of 10,000 cUSD per public key, and only the holder of that key can make it:
//       `claim(claimer)` requires a signature by `claimer` on the transaction.
//     - The claim RETURNS the bucket, so the claimer's own transaction decides where it goes:
//       Caravel turns it into a private output.
//
//   ONE CLAIM PER KEY, WITHOUT A GROWING LIST
//     The Caravel Faucet's pattern: each claim mints a receipt NFT whose id is the claimer's public
//     key and burns it at once. A burned NFT's id persists, so a second mint of the same id fails
//     with "Duplicate NFT token id" and the whole transaction rejects.

use tari_template_lib::prelude::*;

/// cUSD has 6 decimals, like tTARI: 1 cUSD = 1,000,000 base units.
const UNIT: u64 = 1_000_000;
/// The whole supply, ever: 1,000,000,000 cUSD.
const SUPPLY: u64 = 1_000_000_000;
/// One claim: 10,000 cUSD.
const CLAIM: u64 = 10_000;

#[template]
mod caravel_test_dollar {
    use super::*;

    pub struct CaravelTestDollar {
        /// Every unclaimed cUSD.
        supply: Vault,
        /// The claim-receipt NFT resource. Only this component can mint or burn it.
        receipts: ResourceAddress,
    }

    impl CaravelTestDollar {
        /// Create cUSD and the faucet holding all of it.
        pub fn new() -> Component<Self> {
            // The receipt rule names THIS component, whose address only exists after `create()`.
            let allocation = CallerContext::allocate_component_address(None);
            let me = allocation.get_address();

            // No owner: a resource owner passes its own mint and burn rules, so with the default
            // (the deployer's key) the deployer could pre-mint someone's receipt and block their claim.
            let receipts = ResourceBuilder::non_fungible()
                .with_token_symbol("CUSDCLAIM")
                .with_owner_rule(OwnerRule::None)
                .mintable(rule!(component(me)), LOCKED)
                .burnable(rule!(component(me)), LOCKED)
                .build();

            let cusd = ResourceBuilder::stealth()
                .with_token_symbol("cUSD")
                .add_metadata("name", "Caravel Test Dollar")
                .with_owner_rule(OwnerRule::None)
                .mintable(rule!(deny_all), LOCKED)
                .burnable(rule!(deny_all), LOCKED)
                .recallable(rule!(deny_all), LOCKED)
                .freezable(rule!(deny_all), LOCKED)
                .initial_supply(Amount::from(SUPPLY) * Amount::from(UNIT));

            let access_rules = ComponentAccessRules::new()
                .add_method_rule("claim", rule!(allow_all))
                .add_method_rule("remaining", rule!(allow_all))
                .add_method_rule("resource", rule!(allow_all))
                .default(rule!(deny_all));

            Component::new(Self { supply: Vault::from_bucket(cusd), receipts })
                .with_address_allocation(allocation)
                .with_owner_rule(OwnerRule::None)
                .with_access_rules(access_rules)
                .create()
        }

        /// Claim 10,000 cUSD for `claimer`. The transaction must be signed by `claimer`.
        ///
        /// Call this directly as a transaction instruction: signer badges only reach the top-level call.
        pub fn claim(&self, claimer: RistrettoPublicKeyBytes) -> Bucket {
            // Proof that `claimer` signed this transaction; the engine rejects it otherwise.
            CallerContext::get_signer_proof_for_public_key(claimer).drop();

            let claim = Amount::from(CLAIM) * Amount::from(UNIT);
            assert!(self.supply.balance() >= claim, "The cUSD faucet is empty");

            // One claim per key: a second mint of this id fails with "Duplicate NFT token id".
            ResourceManager::get(self.receipts)
                .mint_non_fungible(NonFungibleId::from_u256(claimer.into_array()), &(), &())
                .burn();

            self.supply.withdraw(claim)
        }

        /// Unclaimed cUSD, in base units.
        pub fn remaining(&self) -> Amount {
            self.supply.balance()
        }

        /// The cUSD resource address.
        pub fn resource(&self) -> ResourceAddress {
            self.supply.resource_address()
        }
    }
}
