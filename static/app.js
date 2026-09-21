const $ = (id) => document.getElementById(id);

function setLight(el, health) {
  if (!el) return;
  el.classList.remove("good", "heal", "dead");
  el.classList.add(health === "good" || health === "heal" || health === "dead" ? health : "dead");
}

function fmt(n) {
  if (n === undefined || n === null || n === "—") return "—";
  return Number(n).toLocaleString("en-US");
}

function esc(s) {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;");
}

function simpleLabel(s) {
  if (s.frozen) return "Frozen";
  if (!s.connected) return "Down";
  if (s.health === "good") return "Well";
  return s.health_label || "Healing";
}

function paintArrival(arr) {
  const max = Math.max(1, ...arr);
  const host = $("arrival-bars");
  host.innerHTML = arr
    .map((v, i) => {
      const h = Math.max(4, Math.round((v / max) * 120));
      const now = i === arr.length - 1 ? " now" : "";
      return `<b class="${now.trim()}" style="height:${h}px"></b>`;
    })
    .join("");
}

function paintDiskWheel(free, total) {
  const usedEl = $("disk-used-arc");
  const freeEl = $("disk-free-arc");
  if (!usedEl || !freeEl) return;
  const used = Math.max(0, total - free);
  if (!total) {
    usedEl.setAttribute("stroke-dasharray", "0 100");
    freeEl.setAttribute("stroke-dasharray", "0 100");
    freeEl.setAttribute("stroke-dashoffset", "0");
    return;
  }
  const usedPct = (used / total) * 100;
  const freePct = (free / total) * 100;
  usedEl.setAttribute("stroke-dasharray", `${usedPct} 100`);
  freeEl.setAttribute("stroke-dasharray", `${freePct} 100`);
  freeEl.setAttribute("stroke-dashoffset", `${-usedPct}`);
}

function paintWheel(inbound, outbound) {
  const total = inbound + outbound;
  const outEl = $("wheel-out");
  const inEl = $("wheel-in");
  if (!outEl || !inEl) return;
  if (!total) {
    outEl.setAttribute("stroke-dasharray", "0 100");
    inEl.setAttribute("stroke-dasharray", "0 100");
    inEl.setAttribute("stroke-dashoffset", "0");
    return;
  }
  const outPct = (outbound / total) * 100;
  const inPct = (inbound / total) * 100;
  outEl.setAttribute("stroke-dasharray", `${outPct} 100`);
  inEl.setAttribute("stroke-dasharray", `${inPct} 100`);
  inEl.setAttribute("stroke-dashoffset", `${-outPct}`);
}

const knownKeysByRpc = new Map();

function feedTileHtml(item) {
  const start = Number(item.start);
  const end = Number(item.end);
  if (item.kind === "chunk" && Number.isFinite(start) && Number.isFinite(end) && start !== end) {
    return `<span>${esc(String(start))}</span><span class="tile-dash">–</span><span>${esc(String(end))}</span>`;
  }
  return `<span>${esc(item.label)}</span>`;
}

function paintFeed(items, waiting, health, rpcAddr) {
  const list = $("feed");
  const known = knownKeysByRpc.get(rpcAddr) || new Set();
  const incoming = new Set(items.map((i) => i.key));
  list.innerHTML = items
    .map((item) => {
      const fresh = !known.has(item.key);
      return `<li class="${item.kind}${fresh ? " enter" : ""}" data-key="${item.key}">${feedTileHtml(item)}</li>`;
    })
    .join("");
  knownKeysByRpc.set(rpcAddr, incoming);
  $("feed-status-label").textContent = waiting ? "Waiting For Tip" : "Live";
  setLight($("feed-light"), waiting ? (health === "dead" ? "dead" : "heal") : "good");
}

function fillPeers(countId, inId, outId, inBar, outBar, s) {
  const inbound = s.inbound ?? 0;
  const outbound = s.outbound ?? 0;
  const peak = Math.max(1, inbound, outbound);
  $(countId).textContent = String(s.peers || 0);
  $(inId).textContent = inbound;
  $(outId).textContent = outbound;
  $(inBar).style.width = `${(inbound / peak) * 100}%`;
  $(outBar).style.width = `${(outbound / peak) * 100}%`;
}

function connectMode(s) {
  if (s.connected) {
    $("connect-label").textContent = "Connected";
    $("connect-hint").textContent = `Using ${s.rpc_addr}.`;
    return;
  }
  if (s.frozen) {
    $("connect-label").textContent = "Frozen";
    $("connect-hint").textContent = s.network && s.network !== "—"
      ? `${s.network} node is running at ${s.rpc_addr}, but RPC is not answering. IBD may be stuck. Last known status is below.`
      : `Something is listening at ${s.rpc_addr}, but RPC is not answering. Last known status is below.`;
    return;
  }
  $("connect-label").textContent = s.health_label || "Node down";
  if (s.show_manual) {
    $("connect-hint").textContent = s.error
      ? "Auto-connect failed. If the node is running, set the address here."
      : "If the node is on, enter the address and connect.";
    return;
  }
  $("connect-hint").textContent = s.error
    ? `No RPC at ${s.rpc_addr || "127.0.0.1:48332"}.`
    : `Waiting for ${s.rpc_addr || "127.0.0.1:48332"}…`;
}

function paintPower(s) {
  const btn = $("node-power");
  if (!btn) return;
  const on = !!s.node_running;
  btn.textContent = s.node_power_label || (on ? "Turn Node Off" : "Turn Node On");
  btn.classList.toggle("is-on", on && !s.node_busy);
  btn.disabled = !!s.node_busy;
  const hint = $("power-hint");
  if (!hint || hint.dataset.sticky === "1") return;
  if (s.node_busy) {
    hint.textContent = on ? "Stopping the node. This console stays here." : "Starting the node.";
  } else if (on) {
    hint.textContent = "Node is on. This button sends SIGTERM first (clean shutdown).";
  } else {
    hint.textContent = "Node is off. This button starts it. The console stays up either way.";
  }
}

function paintPeerTable(rows) {
  const body = $("peer-table");
  if (!rows || !rows.length) {
    body.innerHTML = `<tr><td colspan="3">No Peers</td></tr>`;
    return;
  }
  body.innerHTML = rows
    .map((p) => {
      const addr = esc(p.addr);
      const dir = p.inbound ? "Inbound" : "Outbound";
      return `<tr>
        <td>${addr}</td>
        <td>${dir}</td>
        <td class="actions">
          <button type="button" class="ghost" data-act="disconnect" data-addr="${addr}">Disconnect</button>
          <button type="button" class="ghost" data-act="block" data-addr="${addr}">Block</button>
        </td>
      </tr>`;
    })
    .join("");
}

function paintBanTable(rows) {
  const body = $("ban-table");
  if (!rows || !rows.length) {
    body.innerHTML = `<tr><td colspan="3">None</td></tr>`;
    return;
  }
  body.innerHTML = rows
    .map((b) => {
      const addr = esc(b.address);
      let until = "Permanent";
      if (b.banned_until) {
        const d = new Date(b.banned_until * 1000);
        until = Number.isNaN(d.getTime()) ? String(b.banned_until) : d.toLocaleString();
      }
      return `<tr>
        <td>${addr}</td>
        <td>${esc(until)}</td>
        <td class="actions">
          <button type="button" class="ghost" data-act="unban" data-addr="${addr}">Unban</button>
        </td>
      </tr>`;
    })
    .join("");
}

function paintPresets(rpcAddr, chain) {
  document.querySelectorAll("#network-presets .chip").forEach((btn) => {
    const matchAddr = btn.dataset.rpc === rpcAddr;
    const matchChain = chain && btn.dataset.chain === chain;
    btn.classList.toggle("is-active", matchAddr || (!rpcAddr && matchChain));
  });
}

function apply(s) {
  const health = s.health || "dead";
  setLight($("live-light"), "good");
  setLight($("connect-light"), health);
  setLight($("status-light"), health);
  setLight($("home-status-light"), health);
  setLight($("accept-light"), s.accepting_inbound && s.connected ? "good" : "dead");

  if (s.rpc_addr && document.activeElement !== $("rpc-addr")) {
    $("rpc-addr").value = s.rpc_addr;
  }
  if (s.rpc_addr) syncWalletNodeAddr(s.rpc_addr);
  connectMode(s);
  paintPower(s);

  $("home-status-label").textContent = simpleLabel(s);

  const empty = !s.sync_pct_num || s.sync_pct_num === "—";
  $("sync-num").textContent = empty && !s.local_height ? "0" : (s.sync_pct_num && s.sync_pct_num !== "—" ? s.sync_pct_num : "0");
  $("sync-unit").textContent = "%";
  $("sync-pct").classList.toggle("empty", empty);

  if (s.connected) {
    if (s.behind === 0) $("behind-line").textContent = "At Network Tip";
    else if (s.behind === 1) $("behind-line").textContent = "1 Block Behind Network Tip";
    else $("behind-line").textContent = `${fmt(s.behind)} Blocks Behind Network Tip`;
  } else if (s.frozen) {
    $("behind-line").textContent = s.ibd
      ? "Node Frozen During Sync — Last Known Status Below"
      : "Node RPC Frozen — Last Known Status Below";
  } else if (s.health_label === "Node down" || s.health === "dead") {
    $("behind-line").textContent = "Node Is Down — Console Is Still Here";
  } else if (s.show_manual) {
    $("behind-line").textContent = "Can't Reach The Node";
  } else {
    $("behind-line").textContent = "Looking For The Node";
  }

  const pct = s.network_height ? Math.min(100, (s.local_height / s.network_height) * 100) : 0;
  $("sync-bar").style.width = `${pct}%`;
  $("local-height").textContent = fmt(s.local_height ?? 0);
  $("network-height").textContent = s.network_height ? fmt(s.network_height) : "—";
  $("ibd-status").textContent = s.frozen && s.ibd ? "Frozen" : s.ibd ? "Active" : "Idle";
  $("ibd-status").classList.toggle("accent", !!(s.ibd || s.frozen));

  paintArrival(s.arrival && s.arrival.length ? s.arrival : Array(12).fill(0));
  paintFeed(s.feed || [], !!s.feed_waiting, health, s.rpc_addr || "");

  $("node-status").textContent = s.node_status || "Unreachable";
  $("uptime").textContent = s.uptime || "—";
  $("blvm-version").textContent = s.blvm_version || "—";
  $("network").textContent = s.network || "—";
  $("settings-chain").textContent = s.network && s.network !== "—" ? s.network : "—";

  const mark = $("health-mark");
  mark.classList.remove("good", "heal", "dead");
  mark.classList.add(health);
  const icon = $("health-icon");
  if (health === "dead") {
    icon.setAttribute("d", "M9 9l6 6M15 9l-6 6");
  } else if (health === "heal") {
    icon.setAttribute("d", "M8 12h8");
  } else {
    icon.setAttribute("d", "M8 12.5l2.6 2.6L16.5 9.5");
  }

  $("home-peer-count").textContent = String(s.peers || 0);
  $("home-inbound").textContent = String(s.inbound ?? 0);
  $("home-outbound").textContent = String(s.outbound ?? 0);
  $("home-peer-big").textContent = String(s.peers || 0);
  paintWheel(s.inbound ?? 0, s.outbound ?? 0);

  fillPeers("peer-count", "inbound", "outbound", "in-bar", "out-bar", s);
  $("accept-text").textContent = s.frozen
    ? "RPC Frozen"
    : s.accepting_inbound && s.connected
    ? "Accepting Inbound Connections"
    : "Not Listening";

  paintPeerTable(s.peer_rows || []);
  paintBanTable(s.banned || []);
  paintPresets(s.rpc_addr, s.network);

  const netToggle = $("network-active");
  if (document.activeElement !== netToggle) {
    if (s.frozen) {
      if (s.blvm_version && s.blvm_version !== "—") {
        netToggle.checked = !!s.network_active;
      }
    } else {
      netToggle.checked = !!s.network_active;
    }
  }
  netToggle.disabled = !s.connected;
  netToggle.title = s.frozen ? "RPC frozen — showing last known P2P state" : "";

  $("disk-num").textContent = !s.disk_used_num || s.disk_used_num === "—" ? "0" : s.disk_used_num;
  $("disk-unit").textContent = s.disk_used_unit || "GB";
  $("disk-num").parentElement.classList.toggle("empty", !s.disk_used_bytes);
  const haveVol = Number(s.disk_total_bytes) > 0;
  $("disk-free-num").textContent = haveVol
    ? (!s.disk_free_num || s.disk_free_num === "—" ? "0" : s.disk_free_num)
    : "—";
  $("disk-free-unit").textContent = s.disk_free_unit || "GB";
  $("disk-used-legend").textContent = s.disk_vol_used_label || "—";
  $("disk-free-legend").textContent = s.disk_free_label || "—";
  paintDiskWheel(Number(s.disk_free_bytes) || 0, Number(s.disk_total_bytes) || 0);

  $("ui-version").textContent = s.ui_version || "v0.1.0";
  $("ui-uptime").textContent = s.ui_uptime || "—";
  $("last-check").textContent = s.last_check || "just now";
}

function showTab(name) {
  document.querySelectorAll(".tab").forEach((btn) => {
    btn.classList.toggle("is-active", btn.dataset.tab === name);
  });
  document.querySelectorAll(".panel").forEach((panel) => {
    const on = panel.id === `tab-${name}`;
    panel.classList.toggle("is-active", on);
    panel.hidden = !on;
  });
  if (location.hash !== `#${name}`) {
    history.replaceState(null, "", `#${name}`);
  }
}

function showSub(name) {
  if (name === "wallet") return;
  document.querySelectorAll(".subtab").forEach((btn) => {
    btn.classList.toggle("is-active", btn.dataset.sub === name);
  });
  document.querySelectorAll(".subpanel").forEach((panel) => {
    const on = panel.id === `sub-${name}`;
    panel.classList.toggle("is-active", on);
    panel.hidden = !on;
  });
}

function say(id, text) {
  const el = $(id);
  if (el) el.textContent = text || "";
}

function friendlyErr(msg) {
  const s = String(msg || "");
  if (/^connect /i.test(s) || /connection refused/i.test(s)) {
    return "Can't reach the node.";
  }
  return s || "Could not apply that setting.";
}

async function callSetting(method, params) {
  const r = await fetch("/api/rpc", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ method, params }),
  });
  const j = await r.json().catch(() => ({}));
  if (!r.ok || j.ok === false) {
    throw new Error(friendlyErr(j.error));
  }
  return j;
}

async function connectTo(addr) {
  const r = await fetch("/api/connect", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ rpc: addr }),
    signal: AbortSignal.timeout(4000),
  });
  if (r.ok) {
    apply(await r.json());
  }
}

document.querySelectorAll(".tab").forEach((btn) => {
  btn.addEventListener("click", () => showTab(btn.dataset.tab));
});

document.querySelectorAll(".subtab").forEach((btn) => {
  btn.addEventListener("click", () => {
    if (btn.disabled) return;
    showSub(btn.dataset.sub);
  });
});

const hash = (location.hash || "#home").replace("#", "");
const [tabName, subName] = hash.split("/");
showTab(["home", "insights", "settings"].includes(tabName) ? tabName : "home");
if (tabName === "settings" && ["connection", "peers", "network"].includes(subName)) {
  showSub(subName);
}

async function tick() {
  try {
    const r = await fetch("/api/status", { signal: AbortSignal.timeout(3000) });
    apply(await r.json());
  } catch (_) {
    setLight($("live-light"), "dead");
    $("connect-label").textContent = "Console Unreachable";
    $("behind-line").textContent = "This UI Process Is Down";
    $("home-status-label").textContent = "Down";
  }
}

$("connect-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const rpc = $("rpc-addr").value.trim();
  await connectTo(rpc);
});

$("node-power").addEventListener("click", async () => {
  const btn = $("node-power");
  const hint = $("power-hint");
  if (btn.disabled) return;
  btn.disabled = true;
  hint.dataset.sticky = "1";
  hint.textContent = btn.classList.contains("is-on")
    ? "Stopping the node. This console stays here."
    : "Starting the node.";
  try {
    const r = await fetch("/api/node", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ action: "toggle" }),
    });
    const d = await r.json();
    if (d.status) apply(d.status);
    hint.textContent = d.error || d.message || hint.textContent;
  } catch (err) {
    hint.textContent = err.message || "Power request failed.";
  } finally {
    btn.disabled = false;
    hint.dataset.sticky = "0";
  }
});

$("add-peer-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const addr = $("add-peer-addr").value.trim();
  if (!addr) return;
  try {
    await callSetting("addnode", [addr, "onetry"]);
    $("add-peer-addr").value = "";
    say("peers-msg", "Trying that peer.");
    tick();
  } catch (err) {
    say("peers-msg", err.message);
  }
});

$("ban-form").addEventListener("submit", async (e) => {
  e.preventDefault();
  const addr = $("ban-addr").value.trim();
  if (!addr) return;
  try {
    await callSetting("setban", [addr, "add", 86400]);
    $("ban-addr").value = "";
    say("peers-msg", "Peer blocked for 24 hours.");
    tick();
  } catch (err) {
    say("peers-msg", err.message);
  }
});

$("clear-bans").addEventListener("click", async () => {
  try {
    await callSetting("clearbanned", []);
    say("peers-msg", "All bans cleared.");
    tick();
  } catch (err) {
    say("peers-msg", err.message);
  }
});

$("peer-table").addEventListener("click", async (e) => {
  const btn = e.target.closest("button");
  if (!btn) return;
  const addr = btn.dataset.addr;
  if (!addr) return;
  try {
    if (btn.dataset.act === "disconnect") {
      await callSetting("disconnectnode", [addr]);
      say("peers-msg", "Disconnected.");
    } else if (btn.dataset.act === "block") {
      await callSetting("setban", [addr, "add", 86400]);
      say("peers-msg", "Peer blocked for 24 hours.");
    }
    tick();
  } catch (err) {
    say("peers-msg", err.message);
  }
});

$("ban-table").addEventListener("click", async (e) => {
  const btn = e.target.closest("button");
  if (!btn || btn.dataset.act !== "unban") return;
  const addr = btn.dataset.addr;
  if (!addr) return;
  try {
    await callSetting("setban", [addr, "remove"]);
    say("peers-msg", "Unbanned.");
    tick();
  } catch (err) {
    say("peers-msg", err.message);
  }
});

$("network-active").addEventListener("change", async (e) => {
  try {
    await callSetting("setnetworkactive", [e.target.checked]);
    say("network-msg", e.target.checked ? "Network is active." : "Peer activity paused.");
    tick();
  } catch (err) {
    e.target.checked = !e.target.checked;
    say("network-msg", err.message);
  }
});

$("network-presets").addEventListener("click", async (e) => {
  const btn = e.target.closest(".chip");
  if (!btn) return;
  const addr = btn.dataset.rpc;
  if (!addr) return;
  await connectTo(addr);
  say("network-msg", `Console now targeting ${addr}.`);
});

$("wallet-form").addEventListener("submit", (e) => {
  e.preventDefault();
});

const WALLET_APPS = {
  sparrow: {
    title: "Sparrow",
    steps: [
      "Open Settings, then Server, then Electrum.",
      "Paste this node’s host, port, username, and password.",
      "Leave TLS off unless you terminate TLS in front of this node.",
    ],
  },
  specter: {
    title: "Specter",
    steps: [
      "Add an Electrum node.",
      "Use the host, port, username, and password from this page.",
      "Leave TLS off unless you terminate TLS in front of this node.",
    ],
  },
  "fully-noded": {
    title: "Fully Noded",
    steps: [
      "Add an Electrum node. Paste host, port, username, and password from this page.",
    ],
  },
  bitbox: {
    title: "BitBoxApp",
    steps: [
      "Open Settings, then Connect to your own full node, then Electrum.",
      "Use the same host, port, username, and password as this page.",
      "Leave TLS off unless you terminate TLS in front of this node.",
    ],
  },
};

let walletReady = false;
let qrTimer = 0;

function splitHostPort(addr) {
  if (!addr) return ["127.0.0.1", "48332"];
  if (addr.startsWith("[")) {
    const end = addr.indexOf("]");
    if (end > 0) return [addr.slice(1, end), addr.slice(end + 2) || "48332"];
  }
  const i = addr.lastIndexOf(":");
  if (i <= 0) return [addr, "48332"];
  return [addr.slice(0, i), addr.slice(i + 1)];
}

function fieldValue(el) {
  if (!el) return "";
  return el.tagName === "INPUT" ? el.value : (el.textContent || "");
}

function setFieldValue(el, value) {
  if (!el) return;
  if (el.tagName === "INPUT") el.value = value;
  else el.textContent = value;
}

function syncWalletNodeAddr(rpcAddr) {
  const [host, port] = splitHostPort(rpcAddr);
  const hostEl = $("wallet-host");
  const portEl = $("wallet-port");
  if (!hostEl || !portEl) return;
  const changed = fieldValue(hostEl) !== host || fieldValue(portEl) !== port;
  setFieldValue(hostEl, host);
  setFieldValue(portEl, port);
  if (changed && walletReady) scheduleQr();
}

function walletUri() {
  const user = encodeURIComponent($("wallet-user").value.trim());
  const pass = encodeURIComponent($("wallet-pass").value);
  const host = fieldValue($("wallet-host")).trim() || "127.0.0.1";
  const port = fieldValue($("wallet-port")).trim() || "48332";
  const tls = $("wallet-ssl").checked ? "&tls=true" : "";
  return `btcstandup://${user}:${pass}@${host}:${port}/?label=Commons${tls}`;
}

function paintWalletUri() {
  const el = $("wallet-uri");
  const uri = walletUri();
  setFieldValue(el, uri);
  el.title = uri;
}

function paintWalletHelp() {
  const app = WALLET_APPS[$("wallet-app").value] || WALLET_APPS.sparrow;
  $("wallet-help-title").textContent = app.title;
  const ol = $("wallet-help-steps");
  ol.replaceChildren();
  for (const step of app.steps) {
    const li = document.createElement("li");
    li.textContent = step;
    ol.appendChild(li);
  }
}

const QR_PX = 280;
const QR_HOLE =
  "data:image/svg+xml;charset=utf-8," +
  encodeURIComponent(
    '<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"></svg>'
  );
let walletQr = null;

function qrOptions() {
  return {
    width: QR_PX,
    height: QR_PX,
    type: "svg",
    data: walletUri(),
    image: QR_HOLE,
    margin: 4,
    qrOptions: {
      errorCorrectionLevel: "H",
    },
    dotsOptions: {
      color: "#f7931a",
      type: "square",
    },
    backgroundOptions: {
      color: "#0c0c0c",
    },
    cornersSquareOptions: {
      color: "#f7931a",
      type: "square",
    },
    cornersDotOptions: {
      color: "#f7931a",
      type: "square",
    },
    imageOptions: {
      hideBackgroundDots: true,
      imageSize: 0.4,
      margin: 0,
      crossOrigin: "anonymous",
    },
  };
}

function paintQr() {
  paintWalletUri();
  const host = $("wallet-qr");
  if (!host || typeof QRCodeStyling !== "function") return;
  if (!walletQr) {
    host.innerHTML = "";
    walletQr = new QRCodeStyling(qrOptions());
    walletQr.append(host);
  } else {
    walletQr.update(qrOptions());
  }
  settleQr();
}

function settleQr() {
  const panel = $("sub-wallet");
  if (panel && panel.hidden) return;
  let frames = 0;
  const tick = () => {
    fitQrViewBox();
    placeLogoCard();
    frames += 1;
    if (frames < 24) requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}

function qrModuleBox(svg) {
  let x = Infinity;
  let y = Infinity;
  let x2 = -Infinity;
  let y2 = -Infinity;
  for (const el of svg.children) {
    if (el.tagName === "defs" || el.tagName === "image") continue;
    const w = parseFloat(el.getAttribute("width"));
    const h = parseFloat(el.getAttribute("height"));
    if (w >= QR_PX - 0.5 && h >= QR_PX - 0.5) continue;
    try {
      const b = el.getBBox();
      if (!b.width || !b.height) continue;
      x = Math.min(x, b.x);
      y = Math.min(y, b.y);
      x2 = Math.max(x2, b.x + b.width);
      y2 = Math.max(y2, b.y + b.height);
    } catch (_) {}
  }
  if (!Number.isFinite(x)) return null;
  return { x, y, width: x2 - x, height: y2 - y };
}

function fitQrViewBox() {
  const svg = document.querySelector("#wallet-qr svg");
  if (!svg) return;
  const box = qrModuleBox(svg);
  if (!box) return;
  const pad = 6;
  svg.setAttribute(
    "viewBox",
    `${box.x - pad} ${box.y - pad} ${box.width + 2 * pad} ${box.height + 2 * pad}`
  );
  svg.setAttribute("preserveAspectRatio", "xMidYMid meet");
}

function placeLogoCard() {
  const wrap = $("wallet-qr-wrap");
  const card = $("wallet-qr-logo");
  const svg = wrap && wrap.querySelector("svg");
  const hole = svg && svg.querySelector("image");
  if (!wrap || !card || !svg || !hole) return;
  const wr = wrap.getBoundingClientRect();
  const ir = hole.getBoundingClientRect();
  if (!wr.width || !wr.height || !ir.width || !ir.height) return;
  card.style.left = `${((ir.left - wr.left) / wr.width) * 100}%`;
  card.style.top = `${((ir.top - wr.top) / wr.height) * 100}%`;
  card.style.width = `${(ir.width / wr.width) * 100}%`;
  card.style.height = `${(ir.height / wr.height) * 100}%`;
}

function scheduleQr() {
  clearTimeout(qrTimer);
  qrTimer = setTimeout(paintQr, 180);
}

async function loadWalletDefaults() {
  try {
    const d = await (await fetch("/api/wallet-defaults")).json();
    if (d.host) setFieldValue($("wallet-host"), d.host);
    if (d.port) setFieldValue($("wallet-port"), d.port);
    if (d.username) $("wallet-user").value = d.username;
    if (d.password) $("wallet-pass").value = d.password;
    $("wallet-ssl").checked = !!d.tls;
  } catch (_) {}
  walletReady = true;
  paintWalletHelp();
  paintQr();
}

["wallet-user", "wallet-pass"].forEach((id) => {
  $(id).addEventListener("input", scheduleQr);
});
$("wallet-ssl").addEventListener("change", scheduleQr);
$("wallet-app").addEventListener("change", paintWalletHelp);

$("wallet-pass-toggle").addEventListener("click", () => {
  const el = $("wallet-pass");
  const show = el.type === "password";
  el.type = show ? "text" : "password";
  $("wallet-pass-toggle").setAttribute("aria-label", show ? "Hide password" : "Show password");
});

let copyHintTimer = 0;

function flashCopied(btn, ok) {
  document.querySelectorAll(".copy-hint").forEach((el) => el.remove());
  document.querySelectorAll(".field-wrap.has-hint").forEach((el) => {
    el.classList.remove("has-hint");
  });
  const wrap = btn.closest(".field-wrap");
  if (!wrap) return;
  const hint = document.createElement("span");
  hint.className = "copy-hint";
  hint.textContent = ok ? "Copied" : "Couldn’t Copy";
  wrap.classList.add("has-hint");
  wrap.appendChild(hint);
  clearTimeout(copyHintTimer);
  copyHintTimer = setTimeout(() => {
    hint.remove();
    wrap.classList.remove("has-hint");
  }, 2000);
}

$("wallet-form").addEventListener("click", async (e) => {
  const btn = e.target.closest("[data-copy]");
  if (!btn) return;
  const el = $(btn.dataset.copy);
  if (!el) return;
  try {
    await navigator.clipboard.writeText(fieldValue(el));
    flashCopied(btn, true);
  } catch (_) {
    flashCopied(btn, false);
  }
});

loadWalletDefaults();

tick();
setInterval(tick, 1000);
