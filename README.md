# Caravel Trade

A non-custodial private order book on the Tari Ootle.

tTARI vs Caravel Test Dollar (cUSD, testnet placeholder).

1% fee per trade: 0.7% Caravel, 0.3% burned in the Caravel Burn Wallet.

> **Testnet only.** Everything here runs on the Ootle esmeralda testnet (Ootle 0.47, protocol V2).
> tTARI and cUSD have no value, and the contracts are not designed for value. Work in progress.

## Live deployment (esmeralda)

Published and created on 2026-10-14 by the deployer key
`20db90bffb62905d14b75369de9de8523d859bdec5f75e36929fbf9099781661`
(`tari_template_lib` 0.34, `tari_ootle_template_build` 0.15, `tari-ootle-cli` 0.31, walletd 0.46.1).

The network is now on Ootle 0.47, and the source here builds with `tari_template_lib` 0.35,
`tari_ootle_template_build` 0.16 and `tari_template_test_tooling` 0.47. The deployed templates are still
the 0.46.1 builds above; nothing was republished. All three were checked live on 0.47: their on-chain
binaries import only `env.tari_engine` and `env.on_panic`, and dry runs of a place and a faucet claim were
accepted.

### Templates

| Template | Address | Source | Publish tx | Fee (µT) |
|---|---|---|---|---|
| cUSD + faucet (`CaravelTestDollar`) | `template_0338a252870a68f67b157ddee96045282094c5d55691fab82c25d85f9e188c0d` | [`cd6b8c8`](https://github.com/okansaglam016161-pixel/caravel-trade/commit/cd6b8c8ec6d73a60d9c866262bb8818ce5a14d99) | `48d211f9e5ddec52a9c026aeca9a6d971322874eb242317c0e5b054f697b04c3` | 897,722 |
| Board (`CaravelTradeBoard`) | `template_6e9bb8eba68c45e3b5301e1f5ce2e350927cfbffc392c6ed39a4ddb550b8834f` | [`9cc9efb`](https://github.com/okansaglam016161-pixel/caravel-trade/commit/9cc9efb7c7934ffdbc9e1413df1f5763b3def439) | `7911dcf573a66bfc44227781ca5251e910a186f2381041867629a785b8810a94` | 1,278,114 |
| Order (`CaravelTradeOrder`) | `template_73aae2b35810bd46f471986baf797517a00b6a45e9eef924156b2edc4ae8d6d7` | [`9cc9efb`](https://github.com/okansaglam016161-pixel/caravel-trade/commit/9cc9efb7c7934ffdbc9e1413df1f5763b3def439) | `bcc1888c069ec3ab1002b2001da6013b0c0506ffb2e93182c87693e98ed241be` | 2,121,231 |

All three are Commit on both public indexers. The on-chain binaries (identical on both indexers):

| Template | Bytes | sha256 | Metadata hash |
|---|---|---|---|
| cUSD + faucet | 118,579 | `db3bb906935e6d52654ce53e9d026fe112be366e2931fb1f6f417b98fd3d31fb` | `122018414796b94b664fed14387bc037195a7458bef4552fcf894ee0fb0d899a672e` |
| Board | 156,259 | `751bd7b4a2cac06d3ed5711b953eb74e05f31195344e6671453845dafdd148c8` | `1220d3ad2870488ac9a8f87c3d84093832bffb9fdc65421d33d1a7d6df6577cf8954` |
| Order | 201,698 | `8046a3ee143adbf3f0b37cb8a48ac413772cd1c49160b6a24003d5c2a516c325` | `12205a54b19ccbc84d826f41888270e8aadd52e44b43dcd9dff596accbff47e11501` |

### Components and resources

| | Address | Owner |
|---|---|---|
| **cUSD faucet** | `component_d14e2e910e930ce954a5764921bebfaadf0deda354f7a701bc30dd559c15fe0b` | none |
| **cUSD** (resource) | `resource_d1f7fb8d5a70b5a7c7aeb98aefd73d3f87bae5625acfe6f39a072f4c5ba4edea` | none |
| cUSD claim receipts (resource) | `resource_d10371e4de1461659b9dcced72fb46f03642e9d99bf785dc58b817f1a8201446` | none |
| **Board** | `component_278c4da6fd49c03d2735355241bbb29e01748712e2377657769bfbcc365c3bdc` | the deployer key |
| Board badge (resource) | `resource_279ed778dc054969303f7bec1cdc4db2fa410ef22238f98350f373f36faceb43` | none |
| Caravel fee account | `component_4c7dddbe61ec6e77430cc4398535d29e127c1f6d46be4500ce8f2ab164e422bc` | the deployer's account |
| Caravel Burn Wallet | `component_2e91fb78b73440dd114bdbb887d256d760fa3be3276d7b833f4ab1b23d796029` | none ([caravel-burn-wallet](https://github.com/okansaglam016161-pixel/caravel-burn-wallet)) |

Created with [`scripts/instantiate.py`](scripts/instantiate.py), dry run first, final Accept required:
the faucet in `815ff65a5e5bdc40df35ba88dd33d0be9621d8c3f5da088d281330a2eac0a172` (fee 3,249 µT), the Board in
`3c533c12f631a03fce5d0da7a1423ccd1b600c44d6888b4657269df02c5d68ff` (fee 3,095 µT). Orders are created by the
Board, one component per order, each with no owner.

Read live on both indexers after creation:

* **cUSD:** a stealth resource with **6 decimals** (like tTARI) and a total supply of 1,000,000,000 cUSD
  (10^15 base units), all of it in the faucet's vault. Mint, burn, recall and freeze are `DenyAll` and
  `Locked`; the resource has no owner.
* **Faucet:** no owner; `claim`, `remaining` and `resource` are `AllowAll`, everything else `DenyAll`. The
  claim receipts can only be minted and burned by the faucet component itself (`Locked`). A claim is
  10,000 cUSD (10,000,000,000 base units).
* **Board:** owned by the deployer key; the four `place_*` methods and `is_paused` are `AllowAll`,
  everything else (the owner's settings) `DenyAll`. Settings: fee 100 bps (1%), split 70% Caravel / 30%
  Burn Wallet, Caravel fee account as above, Burn Wallet as above, minimum order 1 tTARI, quote token
  cUSD (above), new orders open. The badge has a supply of exactly 1, minting
  `DenyAll` and `Locked`.

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
cargo test                                    # 48 tests: functional, red team, on-chain ABI
```

Against the exact on-chain binaries: download the `binary` field (hex) of `GET /substates/template_<address>` (at `state.up.substate.Template.binary` since Ootle 0.47; `substate.Template.binary` before)
for each template into one directory as `cusd.wasm`, `board.wasm` and `order.wasm`, then

```sh
CARAVEL_TRADE_ONCHAIN=/path/to/dir cargo test
```

The three production templates are then the published bytes, under their published addresses; test
fixtures are still compiled from source. `onchain_abi` checks each on-chain ABI against the source.
On 2026-10-14 all 48 tests passed both ways against the live templates above, and all three ABIs matched.

## Possible upstream bugs

* **The wallet daemon never declares a template address argument as an input.** `transactions.submit_manifest`
  (and `submit` with `detect_inputs`) finds inputs with `IndexedValue::referenced_substates()`
  (`crates/engine_types/src/indexed_value.rs`), which returns only components, resources, NFTs and vaults.
  The engine checks call arguments with `IndexedWellKnownTypes::referenced_substates()`, which also includes
  published template addresses, and refuses one the transaction did not declare: "Template referenced
  substate but it was not found: template_...". So a manifest passing `address!("template_...")` to a
  function always fails through the daemon. A function whose parameter is a `TemplateAddress` (a bare
  `Hash32`) can take `hex_bytes!("...")` instead, which is what `scripts/instantiate.py` does. Present in
  0.46.1 and on `development` as of 2026-10-14. The 0.47.0 changelog lists it fixed in walletd ("every
  referenced substate kind is detected as an input", tari-ootle#2942); not re-tested here, and
  `hex_bytes!` keeps working either way.
