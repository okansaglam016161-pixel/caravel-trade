//   TEST FIXTURE, never published: everything a hostile caller could try against cUSD's supply.

use tari_template_lib::prelude::*;

#[template]
mod caravel_mint_attempt {
    use super::*;

    pub struct CaravelMintAttempt {}

    impl CaravelMintAttempt {
        pub fn mint(resource: ResourceAddress) -> Bucket {
            ResourceManager::get(resource).mint_stealth(Amount::from(1u64))
        }

        pub fn burn(bucket: Bucket) {
            bucket.burn();
        }

        pub fn recall(resource: ResourceAddress, vault: VaultId) -> Bucket {
            ResourceManager::get(resource).recall_all(vault)
        }

        pub fn freeze(resource: ResourceAddress, vault: VaultId) {
            ResourceManager::get(resource).freeze_vault(vault);
        }

        pub fn open_minting(resource: ResourceAddress) {
            ResourceManager::get(resource).update_access_rule(ResourceAuthAction::Mint, rule!(allow_all));
        }
    }
}
