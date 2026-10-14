# Caravel Trade

A non-custodial private order book on the Tari Ootle.

tTARI vs Caravel Test Dollar (cUSD, testnet placeholder).

1% fee per trade: 0.7% Caravel, 0.3% burned in the Caravel Burn Wallet.

> **Testnet only.** Everything here runs on the Ootle esmeralda testnet (Ootle 0.46.1, protocol V2).
> tTARI and cUSD have no value, and the contracts are not designed for value. Work in progress.

## Live deployment (esmeralda)

Published and created on 2026-10-14 (epoch 12259) by the deployer key
`20db90bffb62905d14b75369de9de8523d859bdec5f75e36929fbf9099781661`, from commit
[`9cc9efb`](https://github.com/okansaglam016161-pixel/caravel-trade/commit/9cc9efb7c7934ffdbc9e1413df1f5763b3def439)
(`tari_template_lib` 0.34, `tari_ootle_template_build` 0.15, `tari-ootle-cli` 0.31, walletd 0.46.1).

### Templates

| Template | Address | Publish tx | Fee (µT) |
|---|---|---|---|
| cUSD + faucet (`CaravelTestDollar`) | `template_53257e747c3a4dc5714d2782cf775054d535378f229e31f24b07811176403748` | `cc4b357447adcca015cdff7bae7f098ce0994e24793bb39eb384c18612035694` | 897,722 |
| Board (`CaravelTradeBoard`) | `template_6e9bb8eba68c45e3b5301e1f5ce2e350927cfbffc392c6ed39a4ddb550b8834f` | `7911dcf573a66bfc44227781ca5251e910a186f2381041867629a785b8810a94` | 1,278,114 |
| Order (`CaravelTradeOrder`) | `template_73aae2b35810bd46f471986baf797517a00b6a45e9eef924156b2edc4ae8d6d7` | `bcc1888c069ec3ab1002b2001da6013b0c0506ffb2e93182c87693e98ed241be` | 2,121,231 |

All three are Commit on both public indexers. The on-chain binaries (identical on both indexers):

| Template | Bytes | sha256 | Metadata hash |
|---|---|---|---|
| cUSD + faucet | 118,579 | `dfc74b9667fb22c0d927f9730f5672788a070e265b8605a92a56d0163e14b89a` | `1220c461526d1c7e4123cec512638b1aa000c82a9bf314ea96b376da33a13ada096c` |
| Board | 156,259 | `751bd7b4a2cac06d3ed5711b953eb74e05f31195344e6671453845dafdd148c8` | `1220d3ad2870488ac9a8f87c3d84093832bffb9fdc65421d33d1a7d6df6577cf8954` |
| Order | 201,698 | `8046a3ee143adbf3f0b37cb8a48ac413772cd1c49160b6a24003d5c2a516c325` | `12205a54b19ccbc84d826f41888270e8aadd52e44b43dcd9dff596accbff47e11501` |

### Components and resources

| | Address | Owner |
|---|---|---|
| **cUSD faucet** | `component_bb001707be638663cc88e8feee6fc2b7c391a7640d85c9326e1c9b637d5ca416` | none |
| **cUSD** (resource) | `resource_bb1b1fb0d3ccf5cf196d8b754bf0df8ae16a9ec323f823ec1359e60c592a084d` | none |
| cUSD claim receipts (resource) | `resource_bbd7b4e8deb2e7ed73d6d6c02891ac4629f0f4ee5fd7e070dfea2dc6c3042138` | none |
| **Board** | `component_353a1767f7d1f98984120d1ce5adc1bf7f9b58093b3c4d5539fc106fd2e7526d` | the deployer key |
| Board badge (resource) | `resource_35ca7f6f48040187219dd77571931d5db3b56240972f29091d2283e15f67f50d` | none |
| Caravel fee account | `component_4c7dddbe61ec6e77430cc4398535d29e127c1f6d46be4500ce8f2ab164e422bc` | the deployer's account |
| Caravel Burn Wallet | `component_2e91fb78b73440dd114bdbb887d256d760fa3be3276d7b833f4ab1b23d796029` | none ([caravel-burn-wallet](https://github.com/okansaglam016161-pixel/caravel-burn-wallet)) |

Created with [`scripts/instantiate.py`](scripts/instantiate.py), dry run first, final Accept required:
the faucet in `600155edfcb2bd739e7640449bd974cfb444f76c97fa2eddbef9ebc279f537d4` (fee 3,249 µT), the Board in
`1b2c5ac8fc51f7e75c29bca14634664b5881c0ded1595454d57e6eef2d830c91` (fee 3,095 µT). Orders are created by the
Board, one component per order, each with no owner.

Read live on both indexers after creation:

* **cUSD:** a stealth resource with a total supply of 1,000,000,000 cUSD (10^15 base units), all of it in
  the faucet's vault. Mint, burn, recall and freeze are `DenyAll` and `Locked`; the resource has no owner.
* **Faucet:** no owner; `claim`, `remaining` and `resource` are `AllowAll`, everything else `DenyAll`. The
  claim receipts can only be minted and burned by the faucet component itself (`Locked`).
* **Board:** owned by the deployer key; the four `place_*` methods and `is_paused` are `AllowAll`,
  everything else (the owner's settings) `DenyAll`. Settings: fee 100 bps (1%), split 70% Caravel / 30%
  Burn Wallet, Caravel fee account as above, Burn Wallet as above, minimum order 1 tTARI, quote token
  cUSD, new orders open. The badge has a supply of exactly 1, minting `DenyAll` and `Locked`.

**Known issue: cUSD's divisibility is 8, not 6.** The template treats cUSD as having 6 decimals, like tTARI
(10,000 cUSD per claim is 10^10 base units), but never sets the resource's divisibility, so it got the
default of 8. A wallet or explorer that honours divisibility shows the supply as 10,000,000 cUSD and a claim
as 100 cUSD. Fixing it needs a new cUSD template (`.with_divisibility(6)`), a new faucet and a new Board
(the Board's quote token is fixed at creation).

## Admin

[`scripts/admin`](scripts/admin) changes the Board's settings through the deployer's wallet daemon. Every
change is a dry run unless `--submit` is given.

```sh
source ~/tari-tools/deployer.env
scripts/admin show                          # the live settings (read only)
scripts/admin set-fee 100 70                # fee in bps (at most 10,000) and Caravel's share in percent
scripts/admin set-fee-account component_... # Caravel's fee account
scripts/admin set-min 1                     # minimum order, in tTARI
scripts/admin pause                         # stop new orders (existing orders can still be filled or cancelled)
scripts/admin unpause
```

New settings and pauses only affect orders placed afterwards: every order keeps the settings it was placed
with.

## Build and test

```sh
cd tests
cargo test                                    # 47 tests: functional, red team, on-chain ABI
```

Against the exact on-chain binaries: download the `binary` field (hex) of `GET /substates/template_<address>`
for each template into one directory as `cusd.wasm`, `board.wasm` and `order.wasm`, then

```sh
CARAVEL_TRADE_ONCHAIN=/path/to/dir cargo test
```

The three production templates are then the published bytes, under their published addresses; test
fixtures are still compiled from source. `onchain_abi` checks each on-chain ABI against the source.
On 2026-10-14 all 47 tests passed both ways, and all three ABIs matched.

## Possible upstream bugs

* **The wallet daemon never declares a template address argument as an input.** `transactions.submit_manifest`
  (and `submit` with `detect_inputs`) finds inputs with `IndexedValue::referenced_substates()`
  (`crates/engine_types/src/indexed_value.rs`), which returns only components, resources, NFTs and vaults.
  The engine checks call arguments with `IndexedWellKnownTypes::referenced_substates()`, which also includes
  published template addresses, and refuses one the transaction did not declare: "Template referenced
  substate but it was not found: template_...". So a manifest passing `address!("template_...")` to a
  function always fails through the daemon. A function whose parameter is a `TemplateAddress` (a bare
  `Hash32`) can take `hex_bytes!("...")` instead, which is what `scripts/instantiate.py` does. Present in
  0.46.1 and on `development` as of 2026-10-14.
