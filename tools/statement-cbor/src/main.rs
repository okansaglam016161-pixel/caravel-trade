//! Between a `StealthTransferStatement`'s JSON wire form (the SDK's `toCompactJson()`) and the
//! `tari_bor` CBOR an Order stores as its payout. Same Rust type as the template, so the encoding
//! cannot drift from what the contract's `tari_bor::decode` reads back at fill.
//!
//!   statement-cbor            JSON on stdin  -> CBOR hex on stdout   (the `payout` for place_*_private)
//!   statement-cbor --decode   CBOR hex stdin -> JSON on stdout       (read an order's stored payout)

use std::io::Read;

use tari_template_lib_types::stealth::StealthTransferStatement;

fn fail(msg: String) -> ! {
    eprintln!("{msg}");
    std::process::exit(2)
}

fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).expect("read stdin");
    if std::env::args().any(|a| a == "--decode") {
        let hex = input.trim();
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or_else(|e| fail(format!("not hex: {e}"))))
            .collect();
        let statement: StealthTransferStatement =
            tari_bor::decode(&bytes).unwrap_or_else(|e| fail(format!("not a StealthTransferStatement: {e}")));
        println!("{}", serde_json::to_string(&statement).expect("json"));
        return;
    }
    let statement: StealthTransferStatement =
        serde_json::from_str(&input).unwrap_or_else(|e| fail(format!("not a StealthTransferStatement: {e}")));
    let bytes = tari_bor::encode(&statement).expect("encode");
    let back: StealthTransferStatement = tari_bor::decode(&bytes).expect("round trip");
    assert_eq!(back, statement, "CBOR round trip changed the statement");
    println!("{}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>());
}
