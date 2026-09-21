# blvm-ui

Localhost operator console for a [BLVM](https://thebitcoincommons.org/) node.

`blvm-ui` is an **optional** process. It is not consensus and not required to sync. A node runs without it. The console talks to an already-running node over JSON-RPC and serves a single-page dashboard in the browser.

This repository is the UI crate only. The node, protocol, and consensus live in the rest of the Bitcoin Commons stack.

## What it shows

| Tab | Contents |
|-----|----------|
| **Home** | Sync progress, peer mix (inbound / outbound), last ten blocks |
| **Insights** | Health, disk footprint, block-arrival bars, node uptime |
| **Settings** | RPC connection, peers, network, node power |

Status lights:

- **Green** — RPC up, at tip (or healthy), has peers
- **Amber** — catch-up, IBD, no peers, or RPC TCP up but not answering (**Frozen**)
- **Red** — RPC down

The console process does **not** die if the node dies. After missed polls it keeps the last heights, marks the node down, and offers a manual connect field.

## Requirements

- Rust 1.88+ (edition 2021)
- A BLVM node with JSON-RPC listening (Testnet4 default `127.0.0.1:48332`)
- Unix for the Settings **node power** path (`SIGTERM` / `SIGKILL` via `lsof`; RPC `stop` is not used)

The page is HTML, CSS, and JavaScript **embedded in the binary**. There is no Node.js runtime on the machine that runs the console.

## Quick start

```bash
git clone https://github.com/BTCDecoded/blvm-nodeui.git
cd blvm-nodeui
cargo run --release
```

Open [http://127.0.0.1:3847](http://127.0.0.1:3847).

Point at a different RPC (for example Signet on `38332`):

```bash
BLVM_UI_RPC=127.0.0.1:38332 cargo run --release
```

## Configuration

| Variable | Default | Meaning |
|----------|---------|---------|
| `BLVM_UI_LISTEN` | `127.0.0.1:3847` | Dashboard bind address |
| `BLVM_UI_RPC` | `127.0.0.1:48332` | Node JSON-RPC address |

Bind stays loopback by default. Do not expose this HTTP port on a public interface.

## How it talks to the node

```
 browser  ──HTTP──►  blvm-ui (:3847)  ──JSON-RPC batch──►  blvm-node (:48332)
                         │
                         └── static UI is compiled in
                             (index.html, app.css, app.js, fonts, logo)
```

Every **6 seconds** the console POSTs one JSON-RPC/2.0 **batch**:

`getblockchaininfo`, `getpeerinfo`, `getnetworkinfo`, `listbanned`, `uptime`

The node caps **new** RPC TCP connections at 10 per IP per 60 seconds and closes each HTTP response. Polling faster than that (or one socket per method at 1 Hz) hits `Connection rate limit exceeded`. Connect timeout is 800 ms; exchange timeout is 2 s.

- Connect refused → **Down**
- TCP up, no RPC reply → **Frozen** (amber); last chain height and peer counts are kept

`getblockchaininfo.initialblockdownload` is only `true` at height 0 on this node. The console treats `headers − blocks > 0` as still syncing.

## HTTP API

Served on the console bind address, not on the node.

| Method | Path | Purpose |
|--------|------|---------|
| `GET` | `/` | Dashboard |
| `GET` | `/api/status` | Last poll snapshot (JSON) |
| `POST` | `/api/connect` | Body `{ "rpc": "host:port" }` — switch RPC target |
| `POST` | `/api/rpc` | Whitelisted node RPC only (see below) |
| `POST` | `/api/node` | Body `{ "action": "on" \| "off" \| "toggle" }` — start/stop the node process |

Settings may call **only**:

`addnode`, `disconnectnode`, `setban`, `listbanned`, `clearbanned`, `setnetworkactive`

`stop` is **not** on the whitelist. Node power uses `SIGTERM` (this node’s graceful flush), then `SIGKILL` if needed. The UI process is never killed by that path.

## Layout

```
src/
  main.rs       Bind, 6s poll loop, HTTP server
  lib.rs        Crate root
  http.rs       Routes and embedded static assets
  rpc.rs        JSON-RPC client + settings whitelist
  state.rs      Live snapshot and connect / frozen / down
  feed.rs       Latest-blocks tiles (per RPC address)
  node_ctl.rs   SIGTERM / SIGKILL / spawn remembered blvm
static/         Source for embedded HTML, CSS, JS, fonts, logos
module.toml     Pin for a later `blvm load blvm-ui` spawn (not wired yet)
```

## Tests

```bash
cargo test
```

Coverage includes the settings RPC whitelist, status snapshot lights, and block-feed chunking.

## What this crate is not

- Consensus or chain rules — that is `blvm-consensus` / `blvm-node`
- A mining server — [blvm-stratum-v2](https://github.com/BTCDecoded)
- Pool payouts — Commons Pool
- A replacement for Bitcoin Core’s GUI; it is an operator console for BLVM

BLVM is [Bitcoin Commons](https://thebitcoincommons.org/): same chain, same rules — not a new coin and not a fork.

## License

[MIT](LICENSE) © BTCDecoded
