#!/usr/bin/env python3
"""Create the Caravel Trade components on esmeralda through the deployer's wallet daemon.

    source ~/tari-tools/deployer.env                     # exports TARI_WALLET_DAEMON_API_KEY
    python3 scripts/instantiate.py faucet                # dry run only
    python3 scripts/instantiate.py faucet --submit       # dry run, then submit and wait for the result
    python3 scripts/instantiate.py board --faucet component_...  [--submit]

faucet  CaravelTestDollar::new(): cUSD (1,000,000,000, fixed) and the faucet holding all of it. No owner.
board   CaravelTradeBoard::new(owner, order_template, cUSD, caravel_account, burn_wallet), owned by
        the deployer's key, with the Caravel fee account = the deployer's account. The settings start
        at the template's defaults: fee 1%, split 70/30, minimum order 1 tTARI.

Fees are paid from the deployer account's public balance (the daemon's default account), and
anything above the real cost is refunded. The API key is read from the environment and only ever
sent as a bearer token.

Exit status: 0 on Accept; 3 if the transaction was submitted but has no final result yet (it may
still land, so do not resubmit: check it first); 1 on any failure.
"""

import argparse
import json
import os
import sys
import urllib.request

NETWORK = "esmeralda"
INDEXER = "https://ootle-indexer-a.tari.com"

# Published on esmeralda on 2026-10-14 (Ootle 0.46.1): Board and Order from 9cc9efb, cUSD (6 decimals) from cd6b8c8.
CUSD_TEMPLATE = "0338a252870a68f67b157ddee96045282094c5d55691fab82c25d85f9e188c0d"
BOARD_TEMPLATE = "6e9bb8eba68c45e3b5301e1f5ce2e350927cfbffc392c6ed39a4ddb550b8834f"
ORDER_TEMPLATE = "73aae2b35810bd46f471986baf797517a00b6a45e9eef924156b2edc4ae8d6d7"
# The Caravel Burn Wallet (github.com/okansaglam016161-pixel/caravel-burn-wallet).
BURN_WALLET = "component_2e91fb78b73440dd114bdbb887d256d760fa3be3276d7b833f4ab1b23d796029"

# Dry-run fee cap. Anything above the real cost is refunded, so this only has to be enough.
DRY_RUN_MAX_FEE = 1_000_000
# Head-room over the dry run's required fee for the real submission (also refunded if unused).
FEE_MARGIN = 1.10


# walletd 0.47+: a result wait that runs out of time is a JSON-RPC error with this code instead of a
# timed-out result. The transaction is still in flight and may yet land, so it is not a failure.
TRANSACTION_PENDING = 1003
# Exit status for "submitted, no final result yet" (sys.exit with a message exits 1, a failure).
EXIT_PENDING = 3


class RpcError(Exception):
    def __init__(self, method, error):
        super().__init__(f"{method} failed: {error}")
        self.code = error.get("code") if isinstance(error, dict) else None


def rpc(url, key, method, params, raise_codes=()):
    """Call walletd. Any JSON-RPC error exits, except a code in `raise_codes`, which raises RpcError."""
    body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    req = urllib.request.Request(
        url, data=body, headers={"Content-Type": "application/json", "Authorization": f"Bearer {key}"}
    )
    with urllib.request.urlopen(req, timeout=600) as resp:
        out = json.load(resp)
    if out.get("error"):
        err = RpcError(method, out["error"])
        if err.code in raise_codes:
            raise err
        sys.exit(str(err))
    return out["result"]


def still_pending(tx_id, status):
    """Submitted, no final result yet. Not a failure, and never resubmitted: it may still land."""
    print(f"PENDING       {tx_id} has no final result yet ({status}); it may still land.")
    print("              Do not resubmit. Check it later with walletd's transactions.get_result:")
    print(f'              {{"transaction_id": "{tx_id}"}}')
    sys.exit(EXIT_PENDING)


def wait_final(url, key, tx_id):
    """The final result of a submitted transaction, or exit EXIT_PENDING if it has none yet."""
    try:
        waited = rpc(url, key, "transactions.wait_result", {"transaction_id": tx_id, "timeout_secs": 300},
                     raise_codes=(TRANSACTION_PENDING,))
    except RpcError:
        still_pending(tx_id, "the wait timed out")
    # walletd before 0.47 answers a timed-out wait with a result instead of error 1003.
    if waited.get("timed_out") or waited.get("result") is None:
        still_pending(tx_id, f"status {waited['status']}")
    return waited


def substate_value(body):
    """The value of a GET /substates/<id> body, in either indexer shape, or None for a down answer.

    Ootle 0.47 serves it at state.up.substate (0.46: substate). A down answer only comes back for a
    request with include_proof that names a version, which these scripts never send.
    """
    state = body.get("state") if isinstance(body, dict) else None
    if isinstance(state, dict):
        if "up" in state:
            return state["up"]["substate"]
        if "down" in state:
            return None
    return body.get("substate") if isinstance(body, dict) else None


def indexer(path):
    req = urllib.request.Request(INDEXER + path, headers={"User-Agent": "caravel-trade-instantiate"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        return json.load(resp)


def tagged_hex(value):
    """An address from component state. The 0.46 indexer writes it as tagged CBOR JSON,
    {"@cbor": "tag", "tag": n, "value": {"@cbor": "bytes", "hex": ...}}; older ones as a plain hex string."""
    if isinstance(value, dict) and value.get("@cbor") == "tag":
        value = value["value"]
    if isinstance(value, dict) and value.get("@cbor") == "bytes":
        return value["hex"]
    return str(value).split("_")[-1]


def cusd_of(faucet):
    """The cUSD resource address, read from the live faucet: its supply vault's resource."""
    component = substate_value(indexer(f"/substates/{faucet}"))["Component"]
    if component["header"]["template_address"] != CUSD_TEMPLATE:
        sys.exit(f"{faucet} is not a cUSD faucet (template {component['header']['template_address'][:16]})")
    state = component["body"]["state"]  # struct fields in order: [supply, receipts]
    vault = state[0] if isinstance(state, list) else state["supply"]
    vault_id = f"vault_{tagged_hex(vault)}"
    container = substate_value(indexer(f"/substates/{vault_id}"))["Vault"]["resource_container"]
    kind, body = next(iter(container.items()))
    resource = body["address"]
    return resource if resource.startswith("resource_") else f"resource_{resource}"


def created(finalize, template):
    """The one component the transaction created from `template`."""
    accept = finalize["result"].get("Accept")
    if accept is None:
        return None
    found = []
    for substate_id, substate in accept["up_substates"]:
        header = substate["substate"].get("Component", {}).get("header", {})
        if str(substate_id).startswith("component_") and header.get("template_address") == template:
            found.append(substate_id)
    if len(found) != 1:
        sys.exit(f"expected exactly one new component from template {template[:16]}, found {found}")
    return found[0]


def show(finalize, component):
    """What the new component holds: its owner rule and its state."""
    for substate_id, substate in finalize["result"]["Accept"]["up_substates"]:
        if substate_id == component:
            c = substate["substate"]["Component"]
            print(f"  owner rule  {json.dumps(c['header']['owner_rule'])}")
            print(f"  state       {json.dumps(c['body']['state'])[:1500]}")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("what", choices=["faucet", "board"])
    ap.add_argument("--faucet", help="board: the live cUSD faucet component, for the quote token")
    ap.add_argument("--wallet", default="http://localhost:5100/json_rpc")
    ap.add_argument("--account", default="deployer", help="fee account; for the board also the Caravel fee account")
    ap.add_argument("--submit", action="store_true", help="submit for real after the dry run")
    args = ap.parse_args()

    key = os.environ.get("TARI_WALLET_DAEMON_API_KEY")
    if not key:
        sys.exit("TARI_WALLET_DAEMON_API_KEY is not set")

    info = rpc(args.wallet, key, "wallet.get_info", {})
    if info["network"] != NETWORK:
        sys.exit(f"wallet daemon is on {info['network']}, expected {NETWORK}")
    account = rpc(args.wallet, key, "accounts.get", {"name_or_address": args.account})["account"]
    if not account["is_default"]:
        sys.exit(f"{args.account} is not the default account, so it would not be the fee payer")
    account_address = account["component_address"]
    print(f"fee account   {account_address}")

    if args.what == "faucet":
        template = CUSD_TEMPLATE
        text = f"""
use template_{CUSD_TEMPLATE} as CaravelTestDollar;

fn main() {{
    CaravelTestDollar::new();
}}
"""
    else:
        if not args.faucet:
            sys.exit("board needs --faucet component_... (the live cUSD faucet)")
        template = BOARD_TEMPLATE
        cusd = cusd_of(args.faucet)
        # The owner: the deployer account's own key, read from its owner rule on-chain.
        header = substate_value(indexer(f"/substates/{account_address}"))["Component"]["header"]
        owner_pk = header["owner_rule"]["ByPublicKey"]
        print(f"owner         ByPublicKey({owner_pk})")
        print(f"quote (cUSD)  {cusd}")
        print(f"burn wallet   {BURN_WALLET}")
        # OwnerRule is a minicbor enum, encoded as [variant, [fields]]: ByPublicKey is variant 3.
        # The order template goes in as plain bytes: new() takes a TemplateAddress, which is a bare
        # Hash32. `address!("template_...")` would encode a tagged PublishedTemplateAddress, which the
        # engine treats as a substate the transaction must declare, and the wallet daemon never declares
        # one (see the README's "Possible upstream bugs").
        text = f"""
use template_{BOARD_TEMPLATE} as CaravelTradeBoard;

fn main() {{
    CaravelTradeBoard::new(
        cbor!([3, [public_key!("{owner_pk}")]]),
        hex_bytes!("{ORDER_TEMPLATE}"),
        address!("{cusd}"),
        address!("{account_address}"),
        address!("{BURN_WALLET}"),
    );
}}
"""

    def submit(max_fee, dry_run):
        return rpc(args.wallet, key, "transactions.submit_manifest", {
            "manifest": text, "variables": {}, "max_fee": max_fee, "dry_run": dry_run,
        })

    dry = submit(DRY_RUN_MAX_FEE, True)
    finalize = dry["result"]["finalize"]
    component = created(finalize, template)
    if component is None:
        sys.exit(f"dry run did not Accept: {json.dumps(finalize['result'])[:800]}")
    required = dry["required_fees"]
    print(f"dry run       Accept, required fee {required} µT, would create {component}")
    show(finalize, component)
    print("              (a dry run's addresses are not the final ones: they are derived from the transaction)")
    if not args.submit:
        return

    max_fee = int(required * FEE_MARGIN)
    sent = submit(max_fee, False)
    tx_id = sent["transaction_id"]
    print(f"submitted     {tx_id} (max fee {max_fee} µT)")
    waited = wait_final(args.wallet, key, tx_id)
    component = created(waited["result"], template)
    if component is None:
        sys.exit(f"{tx_id} did not Accept: {json.dumps(waited['result']['result'])[:800]}")
    print(f"ACCEPT        status {waited['status']}, fee {waited['final_fee']} µT")
    print(f"component     {component}")
    show(waited["result"], component)
    print(f"transaction   {tx_id}")


if __name__ == "__main__":
    main()
