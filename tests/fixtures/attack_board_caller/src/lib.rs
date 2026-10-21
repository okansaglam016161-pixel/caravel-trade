//   TEST FIXTURE, never published: an outside template attacking the Board and orders through
//   component calls, for the red-team tests.

use tari_template_lib::prelude::*;

#[template]
mod caravel_attack_board_caller {
    use super::*;

    pub struct CaravelAttackBoardCaller {}

    impl CaravelAttackBoardCaller {
        /// Call an owner-only Board method from inside another template.
        pub fn set_fee(board: ComponentAddress) {
            ComponentManager::get(board).invoke("set_fee", args![10_000u16, 100u8]);
        }

        pub fn pause(board: ComponentAddress) {
            ComponentManager::get(board).invoke("pause", args![]);
        }

        /// Cancel an order from inside another template (the cancel key is not this template's).
        pub fn cancel(order: ComponentAddress) -> Bucket {
            ComponentManager::get(order).call("cancel", args![])
        }

        /// Forge an order: the genuine Order template, a proof of `fake_badge_bucket`'s resource,
        /// and the real Board's address written into its state.
        #[allow(clippy::too_many_arguments)]
        pub fn forge(
            order_template: TemplateAddress,
            fake_badge_bucket: Bucket,
            real_board: ComponentAddress,
            escrow: Bucket,
            quote: ResourceAddress,
            p: Amount,
            account: ComponentAddress,
            cancel_key: RistrettoPublicKeyBytes,
        ) -> (ComponentAddress, Bucket) {
            let n = escrow.amount();
            let proof = fake_badge_bucket.create_proof();
            let order: ComponentAddress = TemplateManager::get(order_template).call("create", args![
                proof,
                fake_badge_bucket.resource_address(),
                real_board,
                0u8,
                escrow,
                n,
                p,
                quote,
                0u16,
                0u8,
                account,
                account,
                Bytes::default(),
                vec![account],
                cancel_key
            ]);
            (order, fake_badge_bucket)
        }

        /// A one-token look-alike badge, symbol and all.
        pub fn fake_badge() -> Bucket {
            ResourceBuilder::public_fungible().with_token_symbol("CTBOARD").initial_supply(1u64)
        }
    }
}
