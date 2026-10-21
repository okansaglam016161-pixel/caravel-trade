//! The published binaries against the source: with `CARAVEL_TRADE_ONCHAIN=<dir>` set (see
//! common::template_test), each on-chain template must expose exactly the ABI the source compiles to.
//! Without it there is nothing on-chain to compare, and the test says so and passes.

mod common;

use common::PRODUCTION;
use tari_template_lib::prelude::TemplateAddress;
use tari_template_test_tooling::Package;

#[test]
fn the_on_chain_abi_is_the_source_abi() {
    let Ok(dir) = std::env::var("CARAVEL_TRADE_ONCHAIN") else {
        println!("CARAVEL_TRADE_ONCHAIN not set: nothing on-chain to compare");
        return;
    };
    for (path, file, address) in PRODUCTION {
        let mut source = Package::builder();
        let source_address = source.add_template(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path));
        let source_def = source.build().get_template_defs().remove(&source_address).expect("source template");

        let code = std::fs::read(std::path::Path::new(&dir).join(file)).expect("on-chain binary");
        let address = TemplateAddress::from_hex(address).expect("address");
        let mut onchain = Package::builder();
        onchain.add_template_from_code(address, code).expect("load the on-chain binary");
        let onchain_def = onchain.build().get_template_defs().remove(&address).expect("on-chain template");

        assert_eq!(format!("{onchain_def:?}"), format!("{source_def:?}"), "{path}: the on-chain ABI differs from the source");
        println!("{path}: on-chain ABI = source ABI ({} functions)", onchain_def.functions().len());
    }
}
