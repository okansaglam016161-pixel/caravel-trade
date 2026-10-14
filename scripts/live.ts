// Caravel Trade, live on esmeralda: the client the trading tests run. Testnet only.
//
//   CARAVEL_TEST_MNEMONIC=... node --import ./scripts/lib/caravel-hooks.mjs scripts/live.ts <command> [--yes]
//
// Commands (every write is priced by a dry run and simulated at its exact fee on both indexers; it is
// only submitted with --yes):
//   status                                         balances: private and public, tTARI and cUSD
//   claim                                          10,000 cUSD from the faucet, straight into a private output
//   reveal-cusd <base units>                       move private cUSD into this wallet's public account
//   place <sell|buy> <n µT> <p cUSD base units> --escrow private|public --payout private|public
//   fill <order> --pay private|public --recv private|public
//   cancel <order>                                 the escrow back to the maker; signed by the order's cancel key
//   race <order>                                   two fills of one order, both pinned to its version, sent together
//   book                                           the open orders, read from the Board's events
//
// The wallet comes from CARAVEL_TEST_MNEMONIC (never printed or written). Private paths are sealed by a
// fresh key and signed only by the one-time keys of the coins they spend, so neither the owner key nor
// the account appears. Public paths sign with the owner key, which is what an account needs.
//
// It reuses Caravel's own wallet code (derivation, the coin scan, fee probing, finality): see
// scripts/lib/caravel-hooks.mjs. Cancel keys are kept in TRADE_STATE (a 600 file), never printed.

import { execFileSync } from 'node:child_process'
import { chmodSync, existsSync, readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  Mask, Network, StealthInput, StealthTransferStatement, TARI_RESOURCE_ADDRESS, TransactionBuilder, WasmStealthCrypto,
  amountLiteral, bytesLiteral, componentAddressLiteral, createOutput, resourceAddressLiteral, decryptOwnedUtxo, generateSealKeypair,
  publicKeyLiteral, sealTransaction, serializeUnsignedTx, signBalanceProof, signTransaction, stealthTransferInstruction,
  stealthUtxoSubstateId,
} from '@tari-project/ootle'
import { IndexerProvider } from '@tari-project/ootle-indexer'
import { SecretKeyWallet } from '@tari-project/ootle-secret-key-wallet'
import { generateKeypair } from '@tari-project/ootle-wasm'
import { deriveIdentity, detectScheme } from 'caravel:src/crypto/derivation'
import { setStoreKey } from 'caravel:src/crypto/sessionKey'
import { StaticSigner, scanOwnedUtxos, type OwnedUtxo } from 'caravel:src/crypto/stealthUtxos'
import { probeAccountAddress } from 'caravel:src/crypto/accountRecovery'
import { nextMaxEpoch } from 'caravel:src/crypto/epoch'
import { simulateFee } from 'caravel:src/crypto/feeProbe'
import { awaitFinality } from 'caravel:src/crypto/finality'
import { feesPaid } from 'caravel:src/crypto/conceal'
import { INDEXER_URL, INDEXER_URLS } from 'caravel:src/crypto/indexerConfig'

// ── The live deployment (README) ─────────────────────────────────────────────
const BOARD = 'component_278c4da6fd49c03d2735355241bbb29e01748712e2377657769bfbcc365c3bdc'
const ORDER_TEMPLATE = '73aae2b35810bd46f471986baf797517a00b6a45e9eef924156b2edc4ae8d6d7'
const FAUCET = 'component_d14e2e910e930ce954a5764921bebfaadf0deda354f7a701bc30dd559c15fe0b'
const CUSD = 'resource_d1f7fb8d5a70b5a7c7aeb98aefd73d3f87bae5625acfe6f39a072f4c5ba4edea'
const TARI = TARI_RESOURCE_ADDRESS
const SELL = 0, BUY = 1
const STATUS = ['OPEN', 'FILLED', 'CANCELLED']

const HERE = path.dirname(fileURLToPath(import.meta.url))
const CBOR_TOOL = path.join(HERE, '../tools/statement-cbor/target/release/statement-cbor')
const STATE_FILE = process.env.TRADE_STATE ?? path.join(HERE, '../.trade-state.json')
const RESERVE = 80_000n // fee for the pricing dry run; the real one pays the measured cost exactly

const argv = process.argv.slice(2)
const yes = argv.includes('--yes')
const flag = (name: string) => { const i = argv.indexOf(`--${name}`); return i >= 0 ? argv[i + 1] : undefined }
const crypto = new WasmStealthCrypto(Network.Esmeralda)
const hex = (b: Uint8Array) => Buffer.from(b).toString('hex')
const fromHex = (h: string) => Uint8Array.from(Buffer.from(h, 'hex'))
const short = (s: string) => `${s.slice(0, s.indexOf('_') + 9)}…${s.slice(-6)}`

// ── Indexer reads ────────────────────────────────────────────────────────────

async function getJson(p: string, base = INDEXER_URL): Promise<any> {
  for (const url of [base, ...INDEXER_URLS.filter(u => u !== base)]) {
    try {
      const r = await fetch(url + p, { headers: { 'user-agent': 'caravel-trade' }, signal: AbortSignal.timeout(20_000) })
      if (r.status === 404) return null
      if (r.ok) return await r.json()
    } catch { /* next indexer */ }
  }
  throw new Error(`no indexer answered ${p}`)
}

/** An address in component state: tagged CBOR JSON on the 0.46 indexer, a plain string before. */
function hexOf(v: any): string {
  if (v && typeof v === 'object' && v['@cbor'] === 'tag') v = v.value
  if (v && typeof v === 'object' && v['@cbor'] === 'bytes') return v.hex
  return String(v).split('_').pop()!
}

async function component(id: string) {
  const r = await getJson(`/substates/${id}`)
  if (!r) throw new Error(`${id} not found`)
  return { version: r.version as number, header: r.substate.Component.header, state: r.substate.Component.body.state as any[] }
}

/** Every vault an account holds, by resource. */
async function accountVaults(account: string): Promise<Map<string, { vault: string; amount: bigint }>> {
  const out = new Map<string, { vault: string; amount: bigint }>()
  const r = await getJson(`/substates/${account}`)
  if (!r) return out
  const found = new Set<string>()
  const walk = (v: any) => {
    if (v && typeof v === 'object') {
      if (v['@cbor'] === 'tag' && v.tag === 132) found.add(`vault_${hexOf(v)}`)
      else for (const x of Object.values(v)) walk(x)
    }
  }
  walk(r.substate.Component.body.state)
  for (const vault of found) {
    const s = await getJson(`/substates/${vault}`)
    const [kind, body] = Object.entries(s.substate.Vault.resource_container)[0] as [string, any]
    const amount = BigInt(kind === 'Stealth' ? body.revealed_amount : body.amount)
    out.set(body.address.startsWith('resource_') ? body.address : `resource_${body.address}`, { vault, amount })
  }
  return out
}

async function order(id: string) {
  const c = await component(id)
  const s = c.state
  const payoutHex = hexOf(s[12])
  return {
    id, version: c.version, template: c.header.template_address as string,
    side: Number(s[0]), status: Number(s[1]), board: `component_${hexOf(s[2])}`, badge: `resource_${hexOf(s[3])}`,
    quote: `resource_${hexOf(s[4])}`, n: BigInt(s[5]), p: BigInt(s[6]), fee: BigInt(s[7]),
    caravelCut: BigInt(s[8]), burnCut: BigInt(s[9]), caravelAccount: `component_${hexOf(s[10])}`,
    burnWallet: `component_${hexOf(s[11])}`, payoutStatement: payoutHex === '' ? null : payoutHex,
    payoutAccount: (s[13] as any[]).map(a => `component_${hexOf(a)}`)[0] ?? null,
    escrowVault: `vault_${hexOf(s[14])}`,
  }
}
type Order = Awaited<ReturnType<typeof order>>

async function boardInfo() {
  const c = await component(BOARD)
  const s = c.state
  return {
    quote: `resource_${hexOf(s[1])}`, feeBps: BigInt(s[2]), caravelPct: BigInt(s[3]),
    caravelAccount: `component_${hexOf(s[4])}`, burnWallet: `component_${hexOf(s[5])}`, minOrder: BigInt(s[6]),
    paused: Boolean(s[7]), badgeVault: `vault_${hexOf(s[8])}`, badge: `resource_${hexOf(s[9])}`,
  }
}

// ── The wallet ───────────────────────────────────────────────────────────────

async function identity() {
  const phrase = process.env.CARAVEL_TEST_MNEMONIC
  if (!phrase?.trim()) throw new Error('CARAVEL_TEST_MNEMONIC is not set')
  const detected = detectScheme(phrase)
  const scheme = detected === 'ambiguous' || detected === 'invalid' ? 'cipherseed' : detected
  const { wallet, storeKey, purposeKeys } = await deriveIdentity(phrase, scheme)
  setStoreKey(storeKey)
  const address = await wallet.getAddress()
  const ownerPk = hex(await wallet.getPublicKey())
  const viewSecret = await wallet.getViewSecret()
  const account = await probeAccountAddress(wallet)
  return { wallet, purposeKeys, address, ownerPk, viewSecret, account }
}
type Who = Awaited<ReturnType<typeof identity>>

/** This wallet's private coins of `resource`, each confirmed present on every indexer. */
async function coins(who: Who, resource: string): Promise<OwnedUtxo[]> {
  let list: OwnedUtxo[]
  if (resource === TARI) {
    list = await scanOwnedUtxos(crypto, who.viewSecret, { walletAddress: who.address })
  } else {
    const resHex = resource.replace(/^resource_/, '')
    const rows = new Map<string, any>()
    for (const base of INDEXER_URLS) {
      let from = ''
      for (let page = 0; page < 50; page++) {
        const r = await getJson(`/utxos?resource_address=${resHex}&limit=1000${from}`, base)
        const got: any[] = r?.utxos ?? []
        for (const [c, body] of got) rows.set(c, body)
        if (got.length < 1000) break
        from = `&from_id=${got[got.length - 1][0]}`
      }
    }
    list = []
    for (const [c, body] of rows) {
      const substateId = `utxo_${resHex}_${c}`
      const d = await decryptOwnedUtxo(crypto, who.viewSecret, { version: 0, verified: false, substate: { Utxo: body } } as never, substateId)
      if (!d) continue
      const key = body?.output?.auth?.Key
      list.push({ substateId, commitment: fromHex(c), nonce: fromHex(body.output.output.public_nonce), value: d.value, mask: d.mask, ...(key ? { spendKey: fromHex(key) } : {}) })
    }
  }
  // A coin a lagging listing still shows after it was spent would only build a doomed transaction.
  const live: OwnedUtxo[] = []
  for (const c of list) {
    const present = await Promise.all(INDEXER_URLS.map(async u => (await fetch(`${u}/substates/${c.substateId}`, { headers: { 'user-agent': 'caravel-trade' } })).status === 200))
    if (present.every(Boolean)) live.push(c)
  }
  return live
}

/** The smallest coin worth at least `min`, with a spend key. */
function pick(list: OwnedUtxo[], min: bigint, not: OwnedUtxo[] = []): OwnedUtxo {
  const c = list.filter(u => u.spendKey && u.value >= min && !not.some(n => n.substateId === u.substateId)).sort((a, b) => (a.value < b.value ? -1 : 1))[0]
  if (!c) throw new Error(`no single coin of at least ${min} base units`)
  return c
}

// ── Statements ───────────────────────────────────────────────────────────────

/** Spend `inputs`: reveal `revealed` to `receiver`, the rest back to us as change. */
async function spend(who: Who, resource: string, inputs: OwnedUtxo[], revealed: bigint, receiver: Uint8Array) {
  const total = inputs.reduce((s, i) => s + i.value, 0n)
  const change = total - revealed
  if (change <= 0n) throw new Error('a spend must leave change')
  const { statement: outs, outputMask } = await crypto.generateOutputsStatement(
    [createOutput({ destination: who.address, amount: change, resourceAddress: resource })], { amount: revealed, receiver })
  const ins = await crypto.buildInputsStatement(inputs.map(i => new StealthInput(i.commitment)), 0n)
  const proof = await signBalanceProof(crypto, await crypto.aggregateInputMasks(inputs.map(i => i.mask)), outputMask, ins, outs)
  return { st: new StealthTransferStatement(ins, outs, proof), outs, change }
}

/** A revealed bucket of exactly `amount` into one private output of ours. */
async function receive(who: Who, resource: string, amount: bigint) {
  const { statement: outs, outputMask } = await crypto.generateOutputsStatement(
    [createOutput({ destination: who.address, amount, resourceAddress: resource })], null)
  const ins = await crypto.buildInputsStatement([], amount)
  const proof = await signBalanceProof(crypto, Mask.zero(), outputMask, ins, outs)
  return new StealthTransferStatement(ins, outs, proof)
}

function commitments(st: StealthTransferStatement): string[] {
  return ((st.outputsStatement as any).parsed().outputs as any[]).map(o => o.output.commitment)
}

/** The change of a spend we are building, decoded as a scan would decode it, so it can be spent too. */
async function decodeOwn(who: Who, resource: string, outs: any, index: number): Promise<OwnedUtxo> {
  const raw = outs.parsed().outputs[index]
  const o = { ...raw, output: { ...raw.output, public_nonce: raw.output.public_nonce ?? raw.output.sender_public_nonce } }
  const substateId = `utxo_${resource.replace(/^resource_/, '')}_${o.output.commitment}`
  const d = await decryptOwnedUtxo(crypto, who.viewSecret, { version: 0, verified: false, substate: { Utxo: { output: o, is_frozen: false } } } as never, substateId)
  if (!d || !o.auth?.Key) throw new Error('could not decode our own change output')
  return { substateId, commitment: fromHex(o.output.commitment), nonce: fromHex(o.output.public_nonce), value: d.value, mask: d.mask, spendKey: fromHex(o.auth.Key) }
}

/** The statement as the bytes an Order stores (tari_bor, via tools/statement-cbor). */
function payoutBytes(st: StealthTransferStatement): Uint8Array {
  return fromHex(execFileSync(CBOR_TOOL, { input: st.toCompactJson() }).toString().trim())
}

// ── Building and sending ─────────────────────────────────────────────────────

type Ins = any
const call = (address: string, method: string, args: any[]): Ins => ({ CallMethod: { call: { Address: address }, method, args } })
const ws = (id: number) => ({ Workspace: { id, offset: null } })

/**
 * One transaction under construction. `fee` is how it pays: privately (a tTARI coin spent in the fee
 * intent, the fee revealed into a bucket that pays it, its change optionally carried on: the chained
 * spend) or publicly (the account pays and is refunded the unused part).
 */
class Tx {
  who: Who
  constructor(who: Who) { this.who = who }
  feeIns: Ins[] = []
  mainIns: Ins[] = []
  inputs = new Map<string, { version: number | null; is_write: boolean }>()
  stealth: OwnedUtxo[] = []
  signers: any[] = []
  keys = [0, 0]
  input(id: string, write = true, version: number | null = null) {
    const prev = this.inputs.get(id)
    this.inputs.set(id, { version: version ?? prev?.version ?? null, is_write: write || (prev?.is_write ?? false) })
  }
  /** Run `ins` and park its output in this intent's workspace; returns the slot. */
  put(intent: 0 | 1, ins: Ins): number {
    const list = intent === 0 ? this.feeIns : this.mainIns
    list.push(ins)
    const k = this.keys[intent]++
    list.push({ PutLastInstructionOutputOnWorkspace: { key: k } })
    return k
  }
  push(intent: 0 | 1, ins: Ins) { (intent === 0 ? this.feeIns : this.mainIns).push(ins) }
}

async function finish(tx: Tx, dryRun: boolean, maxEpoch: number) {
  const b = new TransactionBuilder(Network.Esmeralda, maxEpoch)
  for (const i of tx.feeIns) b.addFeeInstruction(i)
  for (const i of tx.mainIns) b.addInstruction(i)
  for (const [substate_id, d] of tx.inputs) b.addInput({ substate_id, version: d.version, is_write: d.is_write } as never)
  const unsigned = b.buildUnsignedTransaction()
  const seal = generateSealKeypair()
  const json = serializeUnsignedTx(unsigned)
  const sigs = []
  for (const c of tx.stealth) sigs.push(await (tx.who.wallet as any).addStealthSignature(json, c.nonce, seal.public_key, { crypto }))
  const signers = [...tx.signers, ...(sigs.length ? [new StaticSigner(sigs as never)] : [])]
  return sealTransaction(await signTransaction(signers, dryRun ? { ...unsigned, dry_run: true } : unsigned, seal))
}

interface Plan {
  label: string
  /** Builds the transaction for a given fee. Rebuilt for every price, since statements commit to it. */
  build: (fee: bigint) => Promise<{ tx: Tx; notes?: Record<string, string> }>
}

async function provider() { return IndexerProvider.connect({ url: INDEXER_URL, network: Network.Esmeralda }) }

/** Price (dry run), simulate at the exact fee on every indexer, and send when --yes. */
async function run(who: Who, plan: Plan, opts: { submit?: boolean; expectFail?: boolean } = {}) {
  const prov = await provider()
  const maxEpoch = await nextMaxEpoch(prov)
  console.log(`\n── ${plan.label}`)
  const probe = await plan.build(RESERVE)
  const sim0 = await simulateFee(INDEXER_URL, await finish(probe.tx, true, maxEpoch))
  if (!sim0.accepted) {
    console.log(`  dry run REFUSED: ${sim0.message}`)
    if (!opts.expectFail) process.exitCode = 1
    return { refused: sim0.message }
  }
  // Re-price at the fee itself until it holds: a public fee's cost can move with the budget it reserves.
  let fee = sim0.cost
  let real = await plan.build(fee)
  for (let i = 0; i < 3; i++) {
    const again = await simulateFee(INDEXER_URL, await finish(real.tx, true, maxEpoch))
    if (!again.accepted || again.cost === fee) break
    fee = again.cost
    real = await plan.build(fee)
  }
  for (const url of INDEXER_URLS) {
    const s = await simulateFee(url, await finish(real.tx, true, maxEpoch), { fee })
    console.log(`  dry run at exactly ${fee} µT on ${url.replace('https://', '')}: ${s.accepted ? 'Accept' : `REFUSED ${s.message}`}`)
    if (!s.accepted) { process.exitCode = 1; return { refused: s.message } }
  }
  for (const [k, v] of Object.entries(real.notes ?? {})) console.log(`  ${k.padEnd(14)}${v}`)
  console.log(`  fee shown     ${fee} µT`)
  if (!(opts.submit ?? yes)) { console.log('  (dry run only; --yes to send)'); return { fee } }
  const env = await finish(real.tx, false, maxEpoch)
  const sub = await prov.submitTransaction(env as never)
  const txId = sub.transaction_id as string
  const fin = await awaitFinality(prov, txId)
  const paid = feesPaid(fin.body)
  console.log(`  tx            ${txId}`)
  console.log(`  outcome       ${fin.outcome}${fin.reason ? ` (${fin.reason})` : ''}`)
  console.log(`  fee charged   ${paid ?? '?'} µT  ${paid === fee ? '= shown' : '!= shown'}`)
  prov.stopWatcher?.()
  return { txId, fee, paid, outcome: fin.outcome, body: fin.body, notes: real.notes }
}

/** Pay the fee privately: a tTARI coin spent in the fee intent. Returns its change, which the main
 *  intent may spend again in the same transaction without declaring it (the chained spend). */
async function payPrivately(who: Who, tx: Tx, coin: OwnedUtxo, fee: bigint) {
  const f = await spend(who, TARI, [coin], fee, coin.spendKey!)
  const k = tx.put(0, stealthTransferInstruction({ resourceAddress: TARI, revealedInputBucket: null, statement: f.st }, () => ws(0).Workspace))
  tx.push(0, { PayFeeFromBucket: { bucket: { id: k, offset: null } } })
  tx.input(coin.substateId)
  tx.input(TARI, false)
  tx.stealth.push(coin)
  return decodeOwn(who, TARI, f.outs, 0)
}

async function payPublicly(who: Who, tx: Tx, fee: bigint) {
  if (!who.account) throw new Error('this wallet has no account')
  // A budget above the cost: the account is refunded what is not used, so it is charged the cost.
  tx.push(0, call(who.account, 'pay_fee', [amountLiteral(fee + 2_000n)]))
  await declareAccount(tx, who.account)
  tx.signers.push(who.wallet)
}

async function declareAccount(tx: Tx, account: string) {
  tx.input(account)
  for (const [res, v] of await accountVaults(account)) { tx.input(v.vault); tx.input(res, false) }
}

/** `amount` of `resource` from this wallet onto main-intent workspace: revealed from a private coin
 *  (for tTARI, the fee coin's change), or withdrawn from the account. */
async function fund(who: Who, tx: Tx, resource: string, amount: bigint, mode: string, chained: OwnedUtxo | null, spentCusd: OwnedUtxo[]) {
  if (mode === 'public') {
    await declareAccount(tx, who.account!)
    if (!tx.signers.includes(who.wallet)) tx.signers.push(who.wallet)
    return tx.put(1, call(who.account!, 'withdraw', [resourceArg(resource), amountLiteral(amount)]))
  }
  let coin: OwnedUtxo
  if (resource === TARI && chained) {
    coin = chained // never declared: it does not exist until this transaction's fee intent makes it
  } else {
    coin = pick(await coins(who, resource), amount + 1n, spentCusd)
    spentCusd.push(coin)
    tx.input(coin.substateId)
  }
  tx.input(resource, false)
  tx.stealth.push(coin)
  const m = await spend(who, resource, [coin], amount, coin.spendKey!)
  return tx.put(1, stealthTransferInstruction({ resourceAddress: resource, revealedInputBucket: null, statement: m.st }, () => ws(0).Workspace))
}

const resourceArg = (r: string) => resourceAddressLiteral(r as never)

/** Hand the bucket on workspace `k` to this wallet: into the account, or into a new private output. */
async function take(who: Who, tx: Tx, k: number, resource: string, amount: bigint, mode: string) {
  if (mode === 'public') {
    await declareAccount(tx, who.account!)
    tx.push(1, call(who.account!, 'deposit', [ws(k)]))
    return null
  }
  const r = await receive(who, resource, amount)
  tx.input(resource, false)
  const names = new Map([['got', k]])
  tx.push(1, stealthTransferInstruction({ resourceAddress: resource, revealedInputBucket: 'got', statement: r }, n => ({ id: names.get(n)!, offset: null })))
  return r
}

// ── State (cancel keys) ──────────────────────────────────────────────────────

function loadState(): any { return existsSync(STATE_FILE) ? JSON.parse(readFileSync(STATE_FILE, 'utf8')) : { orders: {} } }
function saveState(s: any) { writeFileSync(STATE_FILE, JSON.stringify(s, null, 1)); chmodSync(STATE_FILE, 0o600) }

// ── Commands ─────────────────────────────────────────────────────────────────

async function status(who: Who) {
  const t = await coins(who, TARI), c = await coins(who, CUSD)
  const pub = who.account ? await accountVaults(who.account) : new Map()
  const out = {
    privateTari: t.reduce((s, u) => s + u.value, 0n), privateTariCoins: t.length,
    privateCusd: c.reduce((s, u) => s + u.value, 0n), privateCusdCoins: c.length,
    publicTari: pub.get(TARI)?.amount ?? 0n, publicCusd: pub.get(CUSD)?.amount ?? 0n,
  }
  console.log(`wallet …${who.address.slice(-12)}  account ${who.account ? short(who.account) : 'none'}`)
  for (const [k, v] of Object.entries(out)) console.log(`  ${k.padEnd(18)}${v}`)
  return out
}

async function claim(who: Who) {
  const fstate = (await component(FAUCET)).state
  const vault = `vault_${hexOf(fstate[0])}`, receipts = `resource_${hexOf(fstate[1])}`
  const claimer = await who.purposeKeys.faucet.getPublicKey()
  const tcoins = await coins(who, TARI)
  const X = pick(tcoins, RESERVE * 2n)
  return run(who, {
    label: `claim 10,000 cUSD privately (claimer key ${hex(claimer).slice(0, 8)}…, the wallet's faucet key)`,
    build: async (fee) => {
      const tx = new Tx(who)
      // The fee intent may hold only ONE stealth transfer (the fee's), and a bucket made there cannot be
      // finished in the main intent, so the claim and its private receive run in the main intent. A
      // second claim by the same key is still free: consensus refuses it before it runs, because its
      // receipt NFT already exists ("cannot be created as an output"). A dry run only sees that when the
      // receipt is declared (--declare-receipt).
      const f = await spend(who, TARI, [X], fee, X.spendKey!)
      const kf = tx.put(0, stealthTransferInstruction({ resourceAddress: TARI, revealedInputBucket: null, statement: f.st }, () => ws(0).Workspace))
      tx.push(0, { PayFeeFromBucket: { bucket: { id: kf, offset: null } } })
      const kc = tx.put(1, call(FAUCET, 'claim', [publicKeyLiteral(claimer)]))
      const r = await receive(who, CUSD, 10_000_000_000n)
      tx.push(1, stealthTransferInstruction({ resourceAddress: CUSD, revealedInputBucket: 'c', statement: r }, () => ({ id: kc, offset: null })))
      for (const id of [FAUCET, vault, receipts]) tx.input(id)
      // The claimer's receipt NFT, when asked: declared, an earlier claim's (burnt) receipt is visible
      // to the engine and the mint fails as a duplicate.
      if (argv.includes('--declare-receipt')) tx.input(`nft_${receipts.replace(/^resource_/, '')}_uuid_${hex(claimer)}`, false)
      tx.input(CUSD, false); tx.input(TARI, false); tx.input(X.substateId)
      tx.stealth.push(X)
      tx.signers.push(who.purposeKeys.faucet)
      return { tx, notes: { 'cUSD output': `utxo_…${commitments(r)[0].slice(-12)} (10,000 cUSD)`, 'fee coin': `${short(X.substateId)} (${X.value} µT)` } }
    },
  }, { expectFail: true })
}

async function revealCusd(who: Who, amount: bigint) {
  const X = pick(await coins(who, TARI), RESERVE * 2n)
  return run(who, {
    label: `move ${amount} cUSD base units from private to the account ${short(who.account!)}`,
    build: async (fee) => {
      const tx = new Tx(who)
      await payPrivately(who, tx, X, fee)
      const k = await fund(who, tx, CUSD, amount, 'private', null, [])
      await declareAccount(tx, who.account!)
      tx.push(1, call(who.account!, 'deposit', [ws(k)]))
      return { tx }
    },
  })
}

async function place(who: Who) {
  const side = argv[1] === 'sell' ? SELL : argv[1] === 'buy' ? BUY : (() => { throw new Error('place sell|buy') })()
  const n = BigInt(argv[2]), p = BigInt(argv[3])
  const escrow = flag('escrow') ?? 'private', payout = flag('payout') ?? 'private'
  const board = await boardInfo()
  const fee = n * board.feeBps / 10_000n
  const owed = side === SELL ? p : n - fee
  const kp = generateKeypair()
  const cancelSecret = hex(kp.secret_key), cancelPk = kp.public_key
  // The fee is paid the way the escrow is funded: from a private coin (whose change then funds a
  // tTARI escrow, the chained spend) or from the account.
  const X = escrow === 'private' ? pick(await coins(who, TARI), (side === SELL ? n : 0n) + RESERVE * 2n) : null
  let payoutCommit = ''
  const res = await run(who, {
    label: `place ${side === SELL ? 'SELL' : 'BUY'} ${n} µT tTARI for ${p} cUSD units, escrow ${escrow}, maker paid ${payout}`,
    build: async (feeNow) => {
      const tx = new Tx(who)
      const chained = escrow === 'public' ? (await payPublicly(who, tx, feeNow), null) : await payPrivately(who, tx, X!, feeNow)
      const k = await fund(who, tx, side === SELL ? TARI : CUSD, side === SELL ? n : p, escrow, chained, [])
      let args: any[]
      let method: string
      if (payout === 'private') {
        const st = await receive(who, side === SELL ? CUSD : TARI, owed)
        payoutCommit = commitments(st)[0]
        method = side === SELL ? 'place_sell_private' : 'place_buy_private'
        args = [ws(k), amountLiteral(side === SELL ? p : n), bytesLiteral(payoutBytes(st)), publicKeyLiteral(cancelPk)]
      } else {
        if (!who.account) throw new Error('a public payout needs an account')
        method = side === SELL ? 'place_sell_public' : 'place_buy_public'
        args = [ws(k), amountLiteral(side === SELL ? p : n), componentAddressLiteral(who.account as never), publicKeyLiteral(cancelPk)]
      }
      tx.push(1, call(BOARD, method, args))
      tx.input(BOARD); tx.input(board.badgeVault); tx.input(board.badge, false); tx.input(CUSD, false); tx.input(TARI, false)
      // The new order stores these addresses, and the engine refuses an address it cannot see: read only.
      tx.input(board.caravelAccount, false); tx.input(board.burnWallet, false)
      if (payout === 'public') tx.input(who.account!, false)
      return { tx, notes: { 'payout': payout === 'private' ? `utxo_…${payoutCommit.slice(-12)} (${owed})` : `account ${short(who.account!)} (${owed})` } }
    },
  })
  if (res.txId && res.outcome === 'Commit') {
    const ups: [string, any][] = res.body?.result?.Finalized?.execution_result?.finalize?.result?.Accept?.up_substates ?? []
    const o = ups.find(([id, v]) => id.startsWith('component_') && v?.substate?.Component?.header?.template_address === ORDER_TEMPLATE)?.[0]
    console.log(`  order         ${o}`)
    const s = loadState(); s.orders[o!] = { cancelSecret, maker: who.address.slice(-12), side, n: String(n), p: String(p), escrow, payout, payoutCommit }; saveState(s)
  }
  return res
}

async function fillPlan(who: Who, o: Order, payMode: string, recvMode: string, X: OwnedUtxo | null, pin: boolean, spentCusd: OwnedUtxo[] = []): Promise<Plan> {
  const due = o.side === SELL ? { res: CUSD, amount: o.p } : { res: TARI, amount: o.n }
  const gets = o.side === SELL ? { res: TARI, amount: o.n - o.fee } : { res: CUSD, amount: o.p }
  return {
    label: `fill ${o.side === SELL ? 'SELL' : 'BUY'} ${short(o.id)} (v${o.version}${pin ? ', pinned' : ''}): pay ${due.amount} ${due.res === TARI ? 'µT' : 'cUSD units'} ${payMode}, receive ${gets.amount} ${gets.res === TARI ? 'µT' : 'cUSD units'} ${recvMode}`,
    build: async (fee) => {
      const tx = new Tx(who)
      const used = [...spentCusd]
      const chained = payMode === 'public' ? (await payPublicly(who, tx, fee), null) : await payPrivately(who, tx, X!, fee)
      const k = await fund(who, tx, due.res, due.amount, payMode, chained, used)
      const kg = tx.put(1, call(o.id, 'fill', [ws(k), amountLiteral(o.n), amountLiteral(o.p)]))
      const r = await take(who, tx, kg, gets.res, gets.amount, recvMode)
      tx.input(o.id, true, pin ? o.version : null)
      tx.input(o.escrowVault)
      tx.input(CUSD, false); tx.input(TARI, false)
      await declareAccount(tx, o.caravelAccount)
      tx.input(o.burnWallet)
      for (const [, v] of await accountVaults(o.burnWallet)) tx.input(v.vault)
      if (o.payoutAccount) await declareAccount(tx, o.payoutAccount)
      return { tx, notes: r ? { 'taker output': `utxo_…${commitments(r)[0].slice(-12)} (${gets.amount})` } : {} }
    },
  }
}

async function fill(who: Who) {
  const o = await order(argv[1])
  const payMode = flag('pay') ?? 'private', recvMode = flag('recv') ?? 'private'
  // A private payer pays the fee from a coin (its change pays a tTARI side: the chained spend); a
  // public payer from the account. A private receive needs no coin.
  const X = payMode === 'private' ? pick(await coins(who, TARI), (o.side === BUY ? o.n : 0n) + RESERVE * 2n) : null
  return run(who, await fillPlan(who, o, payMode, recvMode, X, flag('pin') !== 'no'))
}

async function cancel(who: Who) {
  const o = await order(argv[1])
  const saved = loadState().orders[o.id]
  if (!saved) throw new Error('no cancel key for this order in TRADE_STATE')
  const cancelKey = SecretKeyWallet.fromSecretKey(fromHex(saved.cancelSecret), Network.Esmeralda)
  const back = o.side === SELL ? { res: TARI, amount: o.n } : { res: CUSD, amount: o.p }
  const mode = flag('to') ?? (saved.escrow === 'public' ? 'public' : 'private')
  const X = mode === 'private' ? pick(await coins(who, TARI), RESERVE * 2n) : null
  return run(who, {
    label: `cancel ${short(o.id)}: ${back.amount} ${back.res === TARI ? 'µT' : 'cUSD units'} back ${mode}`,
    build: async (fee) => {
      const tx = new Tx(who)
      if (mode === 'public') await payPublicly(who, tx, fee); else await payPrivately(who, tx, X!, fee)
      const k = tx.put(1, call(o.id, 'cancel', []))
      await take(who, tx, k, back.res, back.amount, mode)
      tx.input(o.id); tx.input(o.escrowVault); tx.input(back.res, false)
      tx.signers.push(cancelKey)
      return { tx }
    },
  })
}

async function race(who: Who) {
  // Two takers of one order, both pinned to the version they read. Whichever lands first fills it; the
  // other's pinned input is gone, so it is refused before it runs and pays nothing.
  const o = await order(argv[1])
  const prov = await provider()
  const maxEpoch = await nextMaxEpoch(prov)
  const plans = [await fillPlan(who, o, 'public', 'public', null, true)]
  const X = pick(await coins(who, TARI), (o.side === BUY ? o.n : 0n) + RESERVE * 2n)
  plans.push(await fillPlan(who, o, 'private', 'private', X, true))
  const envs = []
  for (const plan of plans) {
    console.log(`\n── RACE taker: ${plan.label}`)
    const fee = (await simulateFee(INDEXER_URL, await finish((await plan.build(RESERVE)).tx, true, maxEpoch))).cost
    if (fee === undefined) throw new Error('dry run refused')
    const real = await plan.build(fee)
    const sim = await simulateFee(INDEXER_URL, await finish(real.tx, true, maxEpoch), { fee })
    console.log(`  dry run at exactly ${fee} µT: ${sim.accepted ? 'Accept' : `REFUSED ${sim.message}`}`)
    envs.push({ fee, env: await finish(real.tx, false, maxEpoch) })
  }
  if (!yes) { console.log('  (dry run only; --yes to send both)'); return }
  const subs = await Promise.allSettled(envs.map(e => prov.submitTransaction(e.env as never)))
  for (const [i, s] of subs.entries()) {
    if (s.status === 'rejected') { console.log(`  taker ${i + 1}: submission refused: ${String(s.reason).slice(0, 300)}`); continue }
    const txId = s.value.transaction_id as string
    const fin = await awaitFinality(prov, txId)
    console.log(`  taker ${i + 1}: ${txId} → ${fin.outcome}${fin.reason ? ` (${String(fin.reason).slice(0, 200)})` : ''}, fee charged ${feesPaid(fin.body) ?? 0} (shown ${envs[i].fee})`)
  }
  prov.stopWatcher?.()
}

async function book() {
  const board = await boardInfo()
  const ev = await getJson(`/transactions/events?substate_id=${BOARD}&limit=1000`)
  const placed = (ev?.events ?? []).filter((e: any) => e[1].topic === 'CaravelTradeBoard.OrderPlaced')
  console.log(`Board ${short(BOARD)}: ${placed.length} OrderPlaced events`)
  const closed = { FILLED: 0, CANCELLED: 0 }
  for (const [txId, e] of placed) {
    const id = String(e.payload.order)
    const o = await order(id)
    // A real order: from the Order template, naming THIS Board and the Board's own badge (which only
    // the Board holds, so only the Board could have created it).
    const legit = o.template === ORDER_TEMPLATE && o.board === BOARD && o.badge === board.badge
    if (o.status !== 0) { closed[STATUS[o.status] as 'FILLED' | 'CANCELLED']++; continue }
    let payoutCheck = o.payoutAccount ? `account ${short(o.payoutAccount)}` : ''
    if (o.payoutStatement) {
      const st = JSON.parse(execFileSync(CBOR_TOOL, ['--decode'], { input: o.payoutStatement }).toString())
      const c: string = st.outputs_statement.outputs[0].commitment ?? st.outputs_statement.outputs[0].output?.commitment
      const resHex = (o.side === SELL ? o.quote : TARI).replace(/^resource_/, '')
      const exists = (await getJson(`/substates/utxo_${resHex}_${c}`)) !== null
      payoutCheck = `private output …${String(c).slice(-12)} ${exists ? 'ALREADY EXISTS (fill would fail)' : 'free'}`
    }
    console.log(`  OPEN ${short(id)} ${o.side === SELL ? 'SELL' : 'BUY '} n=${o.n} p=${o.p} fee=${o.fee} (${o.caravelCut}+${o.burnCut}) ${legit ? 'legit' : 'NOT LEGIT'} · payout ${payoutCheck} · placed in ${txId.slice(0, 8)}`)
  }
  console.log(`  dropped off the book: ${closed.FILLED} filled, ${closed.CANCELLED} cancelled`)
}

/** The fee's two destinations: Caravel's account (tTARI) and the Burn Wallet (vault and running total). */
async function shared() {
  const board = await boardInfo()
  const caravel = (await accountVaults(board.caravelAccount)).get(TARI)?.amount ?? 0n
  const bw = await component(board.burnWallet)
  const burnVault = (await accountVaults(board.burnWallet)).get(TARI)?.amount ?? 0n
  console.log(`  caravelTari       ${caravel}`)
  console.log(`  burnVault         ${burnVault}`)
  console.log(`  burnTotal         ${BigInt(bw.state[1])}`)
}

async function main() {
  const cmd = argv[0]
  if (cmd === 'book') return book()
  if (cmd === 'shared') return shared()
  const who = await identity()
  if (cmd === 'status') return status(who)
  if (cmd === 'claim') return claim(who)
  if (cmd === 'reveal-cusd') return revealCusd(who, BigInt(argv[1]))
  if (cmd === 'place') return place(who)
  if (cmd === 'fill') return fill(who)
  if (cmd === 'cancel') return cancel(who)
  if (cmd === 'race') return race(who)
  throw new Error(`unknown command ${cmd}`)
}

main().catch(e => { console.error('ERROR:', e instanceof Error ? e.message : e); process.exit(1) })
