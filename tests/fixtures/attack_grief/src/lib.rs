//   TEST FIXTURE, never published: red-team griefing helpers for cUSD and the faucet.

use tari_template_lib::prelude::*;

#[template]
mod caravel_attack_grief {
    use super::*;

    pub struct CaravelAttackGrief {}

    impl CaravelAttackGrief {
        /// Claim for `claimer` through a component call instead of a top-level instruction.
        pub fn claim_via(faucet: ComponentAddress, claimer: RistrettoPublicKeyBytes) -> Bucket {
            ComponentManager::get(faucet).call("claim", args![claimer])
        }

        /// Mint and burn the claim receipt of `victim`, so their own claim later hits a duplicate.
        pub fn burn_receipt_of(receipts: ResourceAddress, victim: RistrettoPublicKeyBytes) {
            ResourceManager::get(receipts)
                .mint_non_fungible(NonFungibleId::from_u256(victim.into_array()), &(), &())
                .burn();
        }
    }
}
