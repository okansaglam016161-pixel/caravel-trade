//! The world every functional test runs in: cUSD + faucet, the Board, the Order template, a copy of
//! the Caravel Burn Wallet, a Caravel fee account, and helpers to place, fill and cancel orders from
//! public or private funds. Network fees are off: these tests are about the contracts' rules.

#![allow(dead_code)]

use tari_template_lib::prelude::{Amount, ComponentAddress, OwnerRule, ResourceAddress, TARI_TOKEN};
use tari_template_lib::types::bytes::Bytes;
use tari_template_lib::types::crypto::RistrettoPublicKeyBytes;
use tari_template_lib::types::stealth::StealthTransferStatement;
use tari_template_lib::prelude::TemplateAddress;
use tari_template_test_tooling::{Package, TemplateTest};
use tari_template_test_tooling::byte_type::ToByteType;
use tari_template_test_tooling::crypto::{PublicKey, RistrettoPublicKey, RistrettoSecretKey, SecretKey};
use tari_template_test_tooling::support::spec::{OutputAuthSpec, OutputSpec};
use tari_template_test_tooling::support::stealth::{NO_INPUTS, generate_transfer_data, generate_transfer_data_to};
use tari_template_test_tooling::transaction::builder::MainIntent;
use tari_template_test_tooling::transaction::{Transaction, TransactionBuilder, args};
use tari_template_test_tooling::wallet_crypto::MaskAndValue;

pub const T: u64 = 1_000_000; // 1 tTARI or 1 cUSD, in base units
pub const SELL: u8 = 0;
pub const BUY: u8 = 1;
pub const OPEN: u8 = 0;
pub const FILLED: u8 = 1;
pub const CANCELLED: u8 = 2;

pub type Builder = TransactionBuilder<MainIntent>;

pub fn keypair() -> (RistrettoSecretKey, RistrettoPublicKeyBytes) {
    let s = RistrettoSecretKey::random(&mut rand::rng());
    let p = RistrettoPublicKey::from_secret_key(&s).to_byte_type();
    (s, p)
}

/// A wallet: a funded public account (1,000 tTARI) and its key.
pub struct Party {
    pub account: ComponentAddress,
    pub secret: RistrettoSecretKey,
    pub pk: RistrettoPublicKeyBytes,
}

/// A private coin the party can spend: its mask signs the spend.
pub struct Coin {
    pub resource: ResourceAddress,
    pub mask: RistrettoSecretKey,
    pub value: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Funds {
    Public,
    Private,
}

pub const BOTH: [Funds; 2] = [Funds::Public, Funds::Private];

pub struct Outcome {
    pub error: String,
    pub ups: Vec<String>,
    pub events: Vec<String>,
}

impl Outcome {
    pub fn ok(&self) -> bool {
        self.error.is_empty()
    }

    #[track_caller]
    pub fn expect_ok(&self, what: &str) -> &Self {
        assert!(self.ok(), "{what} failed: {}", self.error);
        self
    }

    #[track_caller]
    pub fn expect_err(&self, what: &str, contains: &str) {
        assert!(!self.ok(), "{what} should have failed");
        assert!(self.error.contains(contains), "{what}: expected an error containing {contains:?}, got: {}", self.error);
    }

    pub fn created(&self, s: &StealthTransferStatement) -> bool {
        let c = s.outputs_statement.outputs[0].commitment().to_string();
        self.ups.iter().any(|id| id.starts_with("utxo_") && id.ends_with(&c))
    }

    pub fn has_event(&self, topic_suffix: &str) -> bool {
        self.events.iter().any(|t| t.ends_with(topic_suffix))
    }
}

/// An order as placed: its address and, for a private payout, the maker's statement.
pub struct Placed {
    pub order: ComponentAddress,
    pub payout: Option<StealthTransferStatement>,
    pub outcome: Outcome,
}

pub struct World {
    pub test: TemplateTest,
    pub board: ComponentAddress,
    pub faucet: ComponentAddress,
    pub cusd: ResourceAddress,
    pub burn_wallet: ComponentAddress,
    pub caravel: Party,
    pub owner: Party,
}

/// The three production templates: their path in a test's template list, the file holding the
/// on-chain binary, and the address it was published at on esmeralda (2026-10-14, Ootle 0.46.1).
pub const PRODUCTION: [(&str, &str, &str); 3] = [
    ("../templates/cusd_faucet", "cusd.wasm", "53257e747c3a4dc5714d2782cf775054d535378f229e31f24b07811176403748"),
    ("../templates/board", "board.wasm", "6e9bb8eba68c45e3b5301e1f5ce2e350927cfbffc392c6ed39a4ddb550b8834f"),
    ("../templates/order", "order.wasm", "73aae2b35810bd46f471986baf797517a00b6a45e9eef924156b2edc4ae8d6d7"),
];

/// Which binaries a test runs against. By default every template is compiled from source, exactly as
/// `TemplateTest::new` does. With `CARAVEL_TRADE_ONCHAIN=<dir>` set, the three production templates
/// are instead THE BYTES PUBLISHED ON-CHAIN (`<dir>/cusd.wasm`, `board.wasm`, `order.wasm`, the
/// `binary` field of `GET /substates/template_<address>`), registered under their published addresses.
/// Test fixtures are always compiled from source.
pub fn template_test<const N: usize>(paths: [&str; N]) -> TemplateTest {
    let Ok(dir) = std::env::var("CARAVEL_TRADE_ONCHAIN") else {
        return TemplateTest::new(env!("CARGO_MANIFEST_DIR"), paths);
    };
    let mut builder = Package::builder();
    builder.add_all_builtin_templates();
    for path in paths {
        match PRODUCTION.iter().find(|(p, _, _)| *p == path) {
            Some((_, file, address)) => {
                let file = std::path::Path::new(&dir).join(file);
                let code = std::fs::read(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
                let address = TemplateAddress::from_hex(address).expect("published template address");
                builder.add_template_from_code(address, code).expect("load the on-chain binary");
            },
            None => {
                builder.add_template(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path));
            },
        }
    }
    // Exactly what TemplateTest::new does after building its package.
    let mut test = TemplateTest::from_package(builder.build());
    test.bootstrap_state();
    test
}

pub fn world() -> World {
    let mut test = template_test([
        "../templates/cusd_faucet",
        "../templates/board",
        "../templates/order",
        "fixtures/burn_wallet",
        "fixtures/board_v2",
        "fixtures/mint_attempt",
    ]);
    let faucet: ComponentAddress = test.call_function("CaravelTestDollar", "new", args![], vec![]);
    let cusd: ResourceAddress = test.call_method(faucet, "resource", args![], vec![]);
    let burn_wallet: ComponentAddress = test.call_function("CaravelBurnWallet", "new", args![], vec![]);
    let caravel = party(&mut test);
    let owner = party(&mut test);
    let order_template = test.get_template_address("CaravelTradeOrder");
    let board: ComponentAddress = test.call_function(
        "CaravelTradeBoard",
        "new",
        args![OwnerRule::ByPublicKey(owner.pk), order_template, cusd, caravel.account, burn_wallet],
        vec![],
    );
    World { test, board, faucet, cusd, burn_wallet, caravel, owner }
}

pub fn party(test: &mut TemplateTest) -> Party {
    let (account, _, secret, pk) = test.create_funded_account_with_keypair();
    Party { account, secret, pk: pk.to_byte_type() }
}

/// A pre-signed private payout of exactly `amount` revealed funds into one output of a fresh key.
pub fn payout(amount: u64) -> (StealthTransferStatement, Bytes) {
    let (_, pk) = keypair();
    let data = generate_transfer_data(NO_INPUTS, Amount::from(amount), [OutputSpec::new(amount, OutputAuthSpec::KeyPath(pk))], Amount::zero());
    let bytes = Bytes::from_vec(tari_bor::encode(&data.statement).unwrap());
    (data.statement, bytes)
}

pub fn encode(s: &StealthTransferStatement) -> Bytes {
    Bytes::from_vec(tari_bor::encode(s).unwrap())
}

/// A private spend of `coin` revealing `reveal` (to `sealer`) and keeping the rest as change.
pub fn spend(coin: &Coin, reveal: u64, sealer: &RistrettoPublicKeyBytes) -> StealthTransferStatement {
    let change = coin.value - reveal;
    let outputs: Vec<u64> = if change > 0 { vec![change] } else { vec![] };
    generate_transfer_data_to([MaskAndValue { mask: coin.mask.clone(), value: coin.value }], Amount::zero(), outputs, Amount::from(reveal), *sealer).statement
}

/// A private receive: a whole revealed bucket into one output.
pub fn receive(amount: u64) -> StealthTransferStatement {
    generate_transfer_data(NO_INPUTS, Amount::from(amount), [OutputSpec::key_path_from_mask(amount)], Amount::zero()).statement
}

impl World {
    pub fn tx(&self) -> Builder {
        self.test.transaction()
    }

    pub fn party(&mut self) -> Party {
        party(&mut self.test)
    }

    /// Run `b` sealed by `who`, with one extra signature per private coin spent.
    pub fn run(&mut self, b: Builder, who: &Party, spent: &[&Coin]) -> Outcome {
        let mut u = b.finish();
        for c in spent {
            u = u.add_signer(&who.pk, &c.mask);
        }
        self.exec(u.seal(&who.secret))
    }

    pub fn exec(&mut self, tx: Transaction) -> Outcome {
        let r = self.test.execute_and_commit_on_success(tx, vec![]);
        let events = r.finalize.events.iter().map(|e| e.topic().to_string()).collect();
        match r.finalize.result.accept() {
            Some(diff) => Outcome { error: String::new(), ups: diff.up_iter().map(|(id, _)| id.to_string()).collect(), events },
            None => Outcome {
                error: r.finalize.any_reject().map(|e| e.to_string()).unwrap_or_else(|| "rejected".into()),
                ups: vec![],
                events,
            },
        }
    }

    /// Claim 10,000 cUSD for `who` and deposit it in their account.
    pub fn claim_to_account(&mut self, who: &Party) -> Outcome {
        let b = self.tx()
            .call_method(self.faucet, "claim", args![who.pk])
            .put_last_instruction_output_on_workspace("c")
            .call_method(who.account, "deposit", args![Workspace("c")]);
        self.run(b, who, &[])
    }

    /// One private coin worth `value`: tTARI from the account, or cUSD from a fresh faucet claim
    /// (the rest of the claim stays in the account).
    pub fn coin(&mut self, who: &Party, resource: ResourceAddress, value: u64) -> Coin {
        let data = generate_transfer_data(NO_INPUTS, Amount::from(value), [value], Amount::zero());
        let mut b = self.tx();
        if resource != TARI_TOKEN && self.balance(who.account, resource) < Amount::from(value) {
            self.claim_to_account(who).expect_ok("cUSD claim");
            b = self.tx();
        }
        b = b.call_method(who.account, "withdraw", args![resource, Amount::from(value)]);
        let b = b.put_last_instruction_output_on_workspace("in").stealth_transfer_with_input_bucket(resource, data.statement, "in");
        self.run(b, who, &[]).expect_ok("making a private coin");
        Coin { resource, mask: data.output_masks[0].clone(), value }
    }

    pub fn balance(&mut self, account: ComponentAddress, resource: ResourceAddress) -> Amount {
        self.test.call_method(account, "balance", args![resource], vec![])
    }

    pub fn burned(&mut self) -> Amount {
        self.test.call_method(self.burn_wallet, "total_deposited", args![], vec![])
    }

    pub fn caravel_tari(&mut self) -> Amount {
        let a = self.caravel.account;
        self.balance(a, TARI_TOKEN)
    }

    /// `amount` of `resource` onto workspace `key`: withdrawn publicly, or revealed from a new coin.
    fn fund(&mut self, b: Builder, who: &Party, funds: Funds, resource: ResourceAddress, amount: u64, key: &str) -> (Builder, Option<Coin>) {
        match funds {
            Funds::Public => {
                if resource != TARI_TOKEN && self.balance(who.account, resource) < Amount::from(amount) {
                    self.claim_to_account(who).expect_ok("cUSD claim");
                }
                (b.call_method(who.account, "withdraw", args![resource, Amount::from(amount)]).put_last_instruction_output_on_workspace(key), None)
            },
            Funds::Private => {
                let coin = self.coin(who, resource, amount + T);
                let b = b.stealth_transfer(resource, spend(&coin, amount, &who.pk)).put_last_instruction_output_on_workspace(key);
                (b, Some(coin))
            },
        }
    }

    /// Place an order: `side`, N tTARI vs P cUSD, escrow from `funding`, the maker paid by `paid`.
    pub fn place(&mut self, maker: &Party, side: u8, n: u64, p: u64, funding: Funds, paid: Funds, cancel: RistrettoPublicKeyBytes) -> Placed {
        let fee = self.board_fee(n);
        let (escrow_res, escrow_amt, owed) = if side == SELL { (TARI_TOKEN, n, p) } else { (self.cusd, p, n - fee) };
        let b = self.tx();
        let (b, coin) = self.fund(b, maker, funding, escrow_res, escrow_amt, "escrow");
        let (payout, b) = match (side, paid) {
            (SELL, Funds::Private) => {
                let (s, bytes) = payout(owed);
                (Some(s), b.call_method(self.board, "place_sell_private", args![Workspace("escrow"), Amount::from(p), bytes, cancel]))
            },
            (SELL, Funds::Public) => (None, b.call_method(self.board, "place_sell_public", args![Workspace("escrow"), Amount::from(p), maker.account, cancel])),
            (_, Funds::Private) => {
                let (s, bytes) = payout(owed);
                (Some(s), b.call_method(self.board, "place_buy_private", args![Workspace("escrow"), Amount::from(n), bytes, cancel]))
            },
            (_, Funds::Public) => (None, b.call_method(self.board, "place_buy_public", args![Workspace("escrow"), Amount::from(n), maker.account, cancel])),
        };
        let spent: Vec<&Coin> = coin.iter().collect();
        let outcome = self.run(b, maker, &spent);
        let order = if outcome.ok() { self.order_in(&outcome) } else { self.board };
        Placed { order, payout, outcome }
    }

    /// The Board's CURRENT fee on `n` (what a new order would snapshot).
    pub fn board_fee(&self, n: u64) -> u64 {
        let bps: u16 = self.test.extract_component_value(self.board, "$.2");
        n * bps as u64 / 10_000
    }

    pub fn order_in(&self, o: &Outcome) -> ComponentAddress {
        let order_template = self.test.get_template_address("CaravelTradeOrder");
        let store = self.test.read_only_state_store();
        for id in o.ups.iter().filter(|id| id.starts_with("component_")) {
            let addr: ComponentAddress = id.parse().unwrap();
            if store.get_component(addr).map(|c| *c.template_address() == order_template).unwrap_or(false) {
                return addr;
            }
        }
        panic!("no order was created")
    }

    /// Fill `order` (N, P as the taker expects them). The taker pays from `paying` and receives into
    /// `receiving`. `pay_amount` is what the taker actually hands over (normally the order's due).
    #[allow(clippy::too_many_arguments)]
    pub fn fill_with(&mut self, taker: &Party, order: ComponentAddress, side: u8, n: u64, p: u64, pay_res: ResourceAddress, pay_amount: u64,
                     paying: Funds, receiving: Funds) -> (Outcome, Option<StealthTransferStatement>) {
        let fee = self.order_fee(order);
        let (recv_res, recv_amt) = if side == SELL { (TARI_TOKEN, n - fee) } else { (self.cusd, p) };
        let b = self.tx();
        let (b, coin) = self.fund(b, taker, paying, pay_res, pay_amount, "pay");
        let b = b.call_method(order, "fill", args![Workspace("pay"), Amount::from(n), Amount::from(p)]).put_last_instruction_output_on_workspace("got");
        let (b, recv) = match receiving {
            Funds::Public => (b.call_method(taker.account, "deposit", args![Workspace("got")]), None),
            Funds::Private => {
                let r = receive(recv_amt);
                (b.stealth_transfer_with_input_bucket(recv_res, r.clone(), "got"), Some(r))
            },
        };
        let spent: Vec<&Coin> = coin.iter().collect();
        (self.run(b, taker, &spent), recv)
    }

    /// The normal fill: pay exactly what the order is due.
    pub fn fill(&mut self, taker: &Party, order: ComponentAddress, paying: Funds, receiving: Funds) -> (Outcome, Option<StealthTransferStatement>) {
        let (side, n, p) = self.terms(order);
        let (res, amt) = if side == SELL { (self.cusd, p) } else { (TARI_TOKEN, n) };
        self.fill_with(taker, order, side, n, p, res, amt, paying, receiving)
    }

    pub fn cancel(&mut self, who: &Party, signer: &RistrettoSecretKey, order: ComponentAddress) -> Outcome {
        let b = self.tx().call_method(order, "cancel", args![]).put_last_instruction_output_on_workspace("back")
            .call_method(who.account, "deposit", args![Workspace("back")]);
        let tx = b.finish().add_signer(&who.pk, signer).seal(&who.secret);
        self.exec(tx)
    }

    // Order state: 0 side, 1 status, 2 board, 3 board_badge, 4 quote, 5 n, 6 p, 7 fee, 8 caravel_cut,
    // 9 burn_cut, 10 caravel_account, 11 burn_wallet, 12 payout_statement, 13 payout_account, 14 escrow.
    pub fn terms(&self, order: ComponentAddress) -> (u8, u64, u64) {
        let side: u8 = self.test.extract_component_value(order, "$.0");
        let n: Amount = self.test.extract_component_value(order, "$.5");
        let p: Amount = self.test.extract_component_value(order, "$.6");
        (side, n.to_u64_checked().unwrap(), p.to_u64_checked().unwrap())
    }

    pub fn status(&self, order: ComponentAddress) -> u8 {
        self.test.extract_component_value(order, "$.1")
    }

    pub fn order_fee(&self, order: ComponentAddress) -> u64 {
        let fee: Amount = self.test.extract_component_value(order, "$.7");
        fee.to_u64_checked().unwrap()
    }

    pub fn order_cuts(&self, order: ComponentAddress) -> (u64, u64) {
        let c: Amount = self.test.extract_component_value(order, "$.8");
        let b: Amount = self.test.extract_component_value(order, "$.9");
        (c.to_u64_checked().unwrap(), b.to_u64_checked().unwrap())
    }

    pub fn as_owner(&mut self, b: Builder) -> Outcome {
        let o = Party { account: self.owner.account, secret: self.owner.secret.clone(), pk: self.owner.pk };
        self.run(b, &o, &[])
    }
}
