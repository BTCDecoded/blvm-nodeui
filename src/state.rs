use crate::feed::{advance_feed, FeedItem};
use crate::node_ctl::{self, LaunchSpec, PowerPhase};
use crate::rpc;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};

pub const HIST_BARS: usize = 12;
/// Auto-poll this many times before the page shows a manual RPC field.
pub const MANUAL_AFTER: u32 = 5;

#[derive(Debug, Clone, Serialize)]
pub struct PeerRow {
    pub addr: String,
    pub inbound: bool,
    pub subver: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BanRow {
    pub address: String,
    pub banned_until: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Snapshot {
    pub connected: bool,
    pub rpc_addr: String,
    pub health: &'static str,
    pub health_label: String,
    pub node_status: String,
    pub uptime: String,
    pub blvm_version: String,
    pub network: String,
    pub sync_pct: String,
    pub sync_pct_num: String,
    pub syncing: bool,
    pub behind: u64,
    pub local_height: u64,
    pub network_height: u64,
    pub ibd: bool,
    pub ibd_label: String,
    pub peers: usize,
    pub inbound: usize,
    pub outbound: usize,
    pub accepting_inbound: bool,
    pub network_active: bool,
    pub peer_rows: Vec<PeerRow>,
    pub banned: Vec<BanRow>,
    pub disk_used_num: String,
    pub disk_used_unit: String,
    pub disk_used_label: String,
    pub disk_used_bytes: u64,
    pub disk_free_num: String,
    pub disk_free_unit: String,
    pub disk_free_label: String,
    pub disk_free_bytes: u64,
    pub disk_total_bytes: u64,
    pub disk_vol_used_label: String,
    pub feed: Vec<FeedItem>,
    pub feed_waiting: bool,
    pub arrival: Vec<u64>,
    pub last_check: String,
    pub ui_version: String,
    pub ui_uptime: String,
    pub rpc_failures: u32,
    pub show_manual: bool,
    pub error: Option<String>,
    pub frozen: bool,
    pub node_running: bool,
    pub node_busy: bool,
    pub node_power_label: String,
}

impl Snapshot {
    pub fn disconnected(rpc_addr: &str) -> Self {
        let vol = disk_volume();
        Self {
            connected: false,
            rpc_addr: rpc_addr.to_string(),
            health: "dead",
            health_label: "Node down".into(),
            node_status: "Down".into(),
            uptime: "—".into(),
            blvm_version: "—".into(),
            network: "—".into(),
            sync_pct: "—".into(),
            sync_pct_num: "—".into(),
            syncing: false,
            behind: 0,
            local_height: 0,
            network_height: 0,
            ibd: false,
            ibd_label: "idle".into(),
            peers: 0,
            inbound: 0,
            outbound: 0,
            accepting_inbound: false,
            network_active: false,
            peer_rows: Vec::new(),
            banned: Vec::new(),
            disk_used_num: "—".into(),
            disk_used_unit: "".into(),
            disk_used_label: "—".into(),
            disk_used_bytes: 0,
            disk_free_num: vol.free_num,
            disk_free_unit: vol.free_unit,
            disk_free_label: vol.free_label,
            disk_free_bytes: vol.free_bytes,
            disk_total_bytes: vol.total_bytes,
            disk_vol_used_label: vol.used_label,
            feed: Vec::new(),
            feed_waiting: true,
            arrival: vec![0; HIST_BARS],
            last_check: "just now".into(),
            ui_version: format!("v{}", env!("CARGO_PKG_VERSION")),
            ui_uptime: "—".into(),
            rpc_failures: 0,
            show_manual: false,
            error: None,
            frozen: false,
            node_running: false,
            node_busy: false,
            node_power_label: "Turn Node On".into(),
        }
    }
}

struct ChainFeed {
    last: Option<u64>,
    feed: VecDeque<FeedItem>,
    arrival: VecDeque<u64>,
    bucket_started: Instant,
    bucket_delta: u64,
    connected_since: Option<Instant>,
    ever_synced: bool,
    last_snapshot: Option<Snapshot>,
}

impl ChainFeed {
    fn new() -> Self {
        Self {
            last: None,
            feed: VecDeque::new(),
            arrival: VecDeque::from(vec![0; HIST_BARS]),
            bucket_started: Instant::now(),
            bucket_delta: 0,
            connected_since: None,
            ever_synced: false,
            last_snapshot: None,
        }
    }
}

pub struct LiveState {
    pub rpc_addr: String,
    pub started: Instant,
    pub fail_streak: u32,
    pub was_connected: bool,
    views: HashMap<String, ChainFeed>,
    pub snapshot: Snapshot,
    pub power_phase: PowerPhase,
    pub launch: Option<LaunchSpec>,
    pub power_lock: Arc<Mutex<()>>,
}

impl LiveState {
    pub fn new(rpc_addr: String) -> Self {
        let snapshot = Snapshot::disconnected(&rpc_addr);
        Self {
            rpc_addr,
            started: Instant::now(),
            fail_streak: 0,
            was_connected: false,
            views: HashMap::new(),
            snapshot,
            power_phase: PowerPhase::Idle,
            launch: None,
            power_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn switch_rpc(&mut self, addr: String) {
        self.rpc_addr = addr.clone();
        self.fail_streak = 0;
        self.was_connected = false;
        self.snapshot = self
            .views
            .get(&addr)
            .and_then(|c| c.last_snapshot.clone())
            .unwrap_or_else(|| Snapshot::disconnected(&addr));
        self.snapshot.rpc_addr = addr;
        stamp_power(self);
    }
}

pub fn stamp_power(g: &mut LiveState) {
    let running = g.snapshot.connected || g.snapshot.frozen;
    g.snapshot.node_running = running;
    g.snapshot.node_busy = g.power_phase != PowerPhase::Idle;
    g.snapshot.node_power_label = match g.power_phase {
        PowerPhase::Stopping => "Stopping…".into(),
        PowerPhase::Starting => "Starting…".into(),
        PowerPhase::Idle if running => "Turn Node Off".into(),
        PowerPhase::Idle => "Turn Node On".into(),
    };
}

fn chain_mut(g: &mut LiveState) -> &mut ChainFeed {
    let rpc = g.rpc_addr.clone();
    g.views.entry(rpc).or_insert_with(ChainFeed::new)
}

pub type Shared = std::sync::Arc<RwLock<LiveState>>;

pub async fn poll_once(state: &Shared) {
    let rpc_addr = { state.read().await.rpc_addr.clone() };
    match refresh(&rpc_addr, state).await {
        Ok(()) => {}
        Err(e) => {
            let mut g = state.write().await;
            if g.rpc_addr != rpc_addr {
                return;
            }
            mark_unreachable(&mut g, e.to_string());
        }
    }
}

async fn refresh(rpc_addr: &str, state: &Shared) -> anyhow::Result<()> {
    let mut answers = rpc::call_batch(
        rpc_addr,
        &[
            "getblockchaininfo",
            "getpeerinfo",
            "getnetworkinfo",
            "listbanned",
            "uptime",
        ],
    )
    .await?;
    let chain = answers
        .get_mut(0)
        .and_then(Option::take)
        .ok_or_else(|| anyhow::anyhow!("getblockchaininfo failed"))?;
    let peers_v = answers
        .get_mut(1)
        .and_then(Option::take)
        .unwrap_or(serde_json::json!([]));
    let net = answers.get_mut(2).and_then(Option::take);

    let blocks = chain.get("blocks").and_then(|v| v.as_u64()).unwrap_or(0);
    let headers = chain
        .get("headers")
        .and_then(|v| v.as_u64())
        .unwrap_or(blocks);
    let peer_list = peers_v.as_array().cloned().unwrap_or_default();
    let peer_tip = peer_list
        .iter()
        .filter_map(|p| {
            p.get("startingheight").and_then(|v| {
                v.as_u64().or_else(|| {
                    v.as_i64()
                        .and_then(|i| if i > 0 { Some(i as u64) } else { None })
                })
            })
        })
        .max()
        .unwrap_or(0);
    let network_height = headers.max(blocks).max(peer_tip);
    let rpc_ibd = chain
        .get("initialblockdownload")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let chain_name = chain
        .get("chain")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let size_on_disk = chain
        .get("size_on_disk")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let node_uptime = answers
        .get_mut(4)
        .and_then(Option::take)
        .and_then(|v| json_u64(&v))
        .map(|s| format_uptime(Duration::from_secs(s)))
        .unwrap_or_else(|| "—".into());

    let peers = peer_list.len();
    let inbound = peer_list
        .iter()
        .filter(|p| p.get("inbound").and_then(|v| v.as_bool()).unwrap_or(false))
        .count();
    let outbound = peers.saturating_sub(inbound);

    let version = net
        .as_ref()
        .and_then(|n| n.get("subversion").and_then(|v| v.as_str()))
        .map(parse_blvm_version)
        .unwrap_or_else(|| "—".into());

    let peer_rows: Vec<PeerRow> = peer_list
        .iter()
        .map(|p| PeerRow {
            addr: p
                .get("addr")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            inbound: p.get("inbound").and_then(|v| v.as_bool()).unwrap_or(false),
            subver: p
                .get("subver")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        })
        .filter(|p| !p.addr.is_empty())
        .collect();

    let banned_v = answers
        .get_mut(3)
        .and_then(Option::take)
        .unwrap_or(serde_json::json!([]));
    let banned: Vec<BanRow> = banned_v
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|b| BanRow {
            address: b
                .get("address")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            banned_until: b.get("banned_until").and_then(|v| v.as_u64()),
        })
        .filter(|b| !b.address.is_empty())
        .collect();

    let network_active = net
        .as_ref()
        .and_then(|n| n.get("networkactive").and_then(|v| v.as_bool()))
        .unwrap_or(true);
    let behind = network_height.saturating_sub(blocks);
    let (pct, sync_pct, sync_pct_num) = format_sync_pct(blocks, network_height, behind);

    let mut g = state.write().await;
    if g.rpc_addr != rpc_addr {
        return Ok(());
    }
    g.fail_streak = 0;
    g.was_connected = true;
    let (ibd, healing, health, health_label, node_status, feed_waiting, feed_items, arrival) = {
        let c = chain_mut(&mut g);
        rewind_chain_if_needed(c, blocks);
        if behind == 0 && network_height > 0 {
            c.ever_synced = true;
        }
        let ibd = is_ibd(rpc_ibd, blocks, network_height, behind, c.ever_synced);
        let healing = behind > 0 && !ibd;
        let (health, health_label, node_status) = if ibd {
            ("heal", "Syncing", "Syncing")
        } else if healing {
            ("heal", "Healing", "Healing")
        } else if peers == 0 {
            ("heal", "No peers", "No peers")
        } else {
            ("good", "Healthy", "Online")
        };
        if c.connected_since.is_none() {
            c.connected_since = Some(Instant::now());
            if c.feed.is_empty() && c.last.is_none() {
                c.arrival = VecDeque::from(vec![0; HIST_BARS]);
                c.bucket_started = Instant::now();
                c.bucket_delta = 0;
            }
        }
        let delta = {
            let ChainFeed { feed, last, .. } = c;
            advance_feed(feed, last, blocks)
        };
        roll_arrival(c, delta);
        let mut arrival: Vec<u64> = c.arrival.iter().copied().collect();
        if arrival.len() == HIST_BARS {
            arrival[HIST_BARS - 1] = c.bucket_delta;
        }
        let feed_waiting = c.feed.is_empty();
        let feed_items: Vec<FeedItem> = c.feed.iter().cloned().collect();
        (
            ibd,
            healing,
            health,
            health_label,
            node_status,
            feed_waiting,
            feed_items,
            arrival,
        )
    };

    let (disk_num, disk_unit, disk_label) = format_bytes_parts(size_on_disk);
    let vol = disk_volume();

    g.snapshot = Snapshot {
        connected: true,
        rpc_addr: rpc_addr.to_string(),
        health,
        health_label: health_label.into(),
        node_status: node_status.into(),
        uptime: node_uptime,
        blvm_version: version,
        network: chain_name,
        sync_pct,
        sync_pct_num,
        syncing: ibd || healing,
        behind,
        local_height: blocks,
        network_height,
        ibd,
        ibd_label: if ibd {
            "active".into()
        } else if healing {
            "healing".into()
        } else {
            "idle".into()
        },
        peers,
        inbound,
        outbound,
        accepting_inbound: true,
        network_active,
        peer_rows,
        banned,
        disk_used_num: disk_num,
        disk_used_unit: disk_unit,
        disk_used_label: disk_label,
        disk_used_bytes: size_on_disk,
        disk_free_num: vol.free_num,
        disk_free_unit: vol.free_unit,
        disk_free_label: vol.free_label,
        disk_free_bytes: vol.free_bytes,
        disk_total_bytes: vol.total_bytes,
        disk_vol_used_label: vol.used_label,
        feed: feed_items,
        feed_waiting,
        arrival,
        last_check: "just now".into(),
        ui_version: format!("v{}", env!("CARGO_PKG_VERSION")),
        ui_uptime: format_uptime(g.started.elapsed()),
        rpc_failures: 0,
        show_manual: false,
        error: None,
        frozen: false,
        node_running: true,
        node_busy: false,
        node_power_label: "Turn Node Off".into(),
    };
    stamp_power(&mut g);
    node_ctl::remember_if_running(rpc_addr, &mut g.launch);
    let snap = g.snapshot.clone();
    chain_mut(&mut g).last_snapshot = Some(snap);
    let _ = pct;
    Ok(())
}

/// Datadir wipe / reindex / reorg: height went backwards. Arrival bars and
/// "already synced" must not keep the previous chain's state. Feed tiles are
/// trimmed in [`advance_feed`].
fn rewind_chain_if_needed(c: &mut ChainFeed, blocks: u64) {
    if c.last.is_some_and(|h| blocks < h) {
        c.arrival = VecDeque::from(vec![0; HIST_BARS]);
        c.bucket_started = Instant::now();
        c.bucket_delta = 0;
        c.ever_synced = false;
        c.connected_since = Some(Instant::now());
    }
}

fn network_from_rpc(rpc: &str) -> String {
    match rpc.rsplit(':').next().unwrap_or("") {
        "38332" => "signet".into(),
        "48332" => "testnet4".into(),
        "8332" => "main".into(),
        "18332" => "test".into(),
        "18443" => "regtest".into(),
        _ => "—".into(),
    }
}

fn mark_unreachable(g: &mut LiveState, err: String) {
    g.fail_streak = g.fail_streak.saturating_add(1);
    let show_manual = g.fail_streak >= MANUAL_AFTER;
    let ui_uptime = format_uptime(g.started.elapsed());
    let frozen = crate::rpc::is_frozen_error(&err);
    let parked = g.views.get(&g.rpc_addr).and_then(|c| c.last_snapshot.clone());
    let (feed, arrival) = {
        let c = chain_mut(g);
        (
            c.feed.iter().cloned().collect::<Vec<_>>(),
            c.arrival.iter().copied().collect::<Vec<_>>(),
        )
    };
    let mut snap = parked.unwrap_or_else(|| Snapshot::disconnected(&g.rpc_addr));
    snap.connected = false;
    snap.frozen = frozen;
    snap.rpc_addr = g.rpc_addr.clone();
    if snap.network == "—" {
        snap.network = network_from_rpc(&g.rpc_addr);
    }
    if frozen {
        snap.health = "heal";
        snap.health_label = "Frozen".into();
        snap.node_status = "Frozen".into();
        snap.ibd_label = if snap.ibd {
            "frozen".into()
        } else {
            snap.ibd_label.clone()
        };
    } else {
        snap.health = "dead";
        snap.health_label = if show_manual {
            "Can't reach node".into()
        } else {
            "Node down".into()
        };
        snap.node_status = "Down".into();
        snap.network_active = false;
        snap.accepting_inbound = false;
    }
    snap.error = Some(err);
    snap.rpc_failures = g.fail_streak;
    snap.show_manual = show_manual;
    snap.ui_uptime = ui_uptime;
    snap.last_check = "just now".into();
    if snap.feed.is_empty() {
        snap.feed = feed;
    }
    snap.feed_waiting = snap.feed.is_empty();
    if snap.arrival.is_empty() {
        snap.arrival = arrival;
    }
    g.snapshot = snap;
    stamp_power(g);
}

fn roll_arrival(c: &mut ChainFeed, delta: u64) {
    c.bucket_delta = c.bucket_delta.saturating_add(delta);
    if c.bucket_started.elapsed() >= Duration::from_secs(60) {
        if c.arrival.len() != HIST_BARS {
            c.arrival = VecDeque::from(vec![0; HIST_BARS]);
        }
        c.arrival.pop_front();
        c.arrival.push_back(c.bucket_delta);
        c.bucket_delta = 0;
        c.bucket_started = Instant::now();
    }
}

/// `/BitcoinCommons:0.1.0/` or `/blvm-node:0.1.0/` → `v0.1.0`
fn parse_blvm_version(sub: &str) -> String {
    let s = sub.trim().trim_matches('/');
    if let Some((_, ver)) = s.rsplit_once(':') {
        let ver = ver.trim().trim_matches('/');
        if !ver.is_empty() {
            return if ver.starts_with('v') {
                ver.to_string()
            } else {
                format!("v{ver}")
            };
        }
    }
    s.to_string()
}

/// First download vs catch-up after a completed sync.
///
/// Node RPC only sets `initialblockdownload` at height 0. Any lag before this
/// console has seen network tip is IBD (Syncing). After tip, lag is Healing.
/// If the console restarts against an almost-synced node, a ≥99.9% chain with
/// at most ~1 day of headers remaining is Healing, not a new IBD.
fn is_ibd(rpc_ibd: bool, blocks: u64, network_height: u64, behind: u64, ever_synced: bool) -> bool {
    if rpc_ibd {
        return true;
    }
    if behind == 0 {
        return false;
    }
    if ever_synced {
        return false;
    }
    if blocks == 0 {
        return true;
    }
    let near_tip =
        network_height > 0 && (blocks as u128).saturating_mul(1000) / (network_height as u128) >= 999;
    !(near_tip && behind <= 144)
}

fn format_sync_pct(blocks: u64, network_height: u64, behind: u64) -> (f64, String, String) {
    let pct = if network_height == 0 {
        0.0
    } else {
        (blocks as f64 / network_height as f64 * 100.0).clamp(0.0, 100.0)
    };
    if network_height == 0 {
        return (0.0, "—".into(), "—".into());
    }
    if behind == 0 && pct >= 100.0 {
        return (100.0, "100%".into(), "100".into());
    }
    if pct > 0.0 && pct < 1.0 {
        let n = format!("{pct:.2}");
        return (pct, format!("{n}%"), n);
    }
    if pct >= 99.0 && pct < 100.0 {
        let n = format!("{pct:.3}");
        return (pct, format!("{n}%"), n);
    }
    let n = format!("{pct:.1}");
    (pct, format!("{n}%"), n)
}

fn json_u64(v: &serde_json::Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_i64().and_then(|i| u64::try_from(i).ok()))
}

fn format_uptime(d: Duration) -> String {
    let s = d.as_secs();
    let days = s / 86400;
    let hours = (s % 86400) / 3600;
    let mins = (s % 3600) / 60;
    if days > 0 {
        format!("{days}d {hours:02}h {mins:02}m")
    } else {
        format!("{hours:02}h {mins:02}m")
    }
}

struct DiskVolume {
    free_num: String,
    free_unit: String,
    free_label: String,
    used_label: String,
    free_bytes: u64,
    total_bytes: u64,
}

fn disk_volume() -> DiskVolume {
    let (total, free) = volume_space();
    let used = total.saturating_sub(free);
    let (free_num, free_unit, free_label) = format_bytes_parts(free);
    let (_, _, used_label) = format_bytes_parts(used);
    DiskVolume {
        free_num,
        free_unit,
        free_label,
        used_label,
        free_bytes: free,
        total_bytes: total,
    }
}

#[cfg(unix)]
fn volume_space() -> (u64, u64) {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let path = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/"));
    let path = path.canonicalize().unwrap_or(path);
    let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else {
        return (0, 0);
    };
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
        return (0, 0);
    }
    let frag = stat.f_frsize as u64;
    if frag == 0 {
        return (0, 0);
    }
    let total = (stat.f_blocks as u64).saturating_mul(frag);
    let available = (stat.f_bavail as u64).saturating_mul(frag);
    (total, available)
}

#[cfg(not(unix))]
fn volume_space() -> (u64, u64) {
    (0, 0)
}

fn format_bytes_parts(n: u64) -> (String, String, String) {
    const TB: f64 = 1_000_000_000_000.0;
    const GB: f64 = 1_000_000_000.0;
    const MB: f64 = 1_000_000.0;
    if n == 0 {
        return ("—".into(), "".into(), "—".into());
    }
    let t = n as f64 / TB;
    if t >= 1.0 {
        let num = format!("{t:.2}");
        return (num.clone(), "TB".into(), format!("{num} TB"));
    }
    let g = n as f64 / GB;
    if g >= 1.0 {
        let num = format!("{g:.1}");
        return (num.clone(), "GB".into(), format!("{num} GB"));
    }
    let num = format!("{:.0}", n as f64 / MB);
    (num.clone(), "MB".into(), format!("{num} MB"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_uptime_from_node_seconds() {
        assert_eq!(format_uptime(Duration::from_secs(0)), "00h 00m");
        assert_eq!(format_uptime(Duration::from_secs(90)), "00h 01m");
        assert_eq!(format_uptime(Duration::from_secs(3661)), "01h 01m");
        assert_eq!(format_uptime(Duration::from_secs(90_061)), "1d 01h 01m");
        assert_eq!(json_u64(&serde_json::json!(3661)), Some(3661));
    }

    #[cfg(unix)]
    #[test]
    fn volume_space_reads_this_disk() {
        let (total, free) = volume_space();
        assert!(total > 0, "statvfs should see this volume");
        assert!(free <= total);
        let vol = disk_volume();
        assert_eq!(vol.total_bytes, total);
        assert_eq!(vol.free_bytes, free);
        assert_ne!(vol.free_label, "—");
    }

    #[test]
    fn parses_bitcoin_commons_subversion() {
        assert_eq!(parse_blvm_version("/BitcoinCommons:0.2.1/"), "v0.2.1");
        assert_eq!(parse_blvm_version("/blvm-node:0.1.0/"), "v0.1.0");
        assert_eq!(parse_blvm_version("v1.0.0"), "v1.0.0");
    }

    #[test]
    fn sync_pct_near_tip_has_three_decimals() {
        let (_, label, num) = format_sync_pct(927_545, 927_551, 6);
        assert_eq!(label, "99.999%");
        assert_eq!(num, "99.999");
    }

    #[test]
    fn sync_pct_early_ibd_has_two_decimals() {
        let (_, label, num) = format_sync_pct(128, 322_544, 322_416);
        assert_eq!(label, "0.04%");
        assert_eq!(num, "0.04");
    }

    #[test]
    fn manual_connect_after_five_misses() {
        assert_eq!(MANUAL_AFTER, 5);
    }

    #[test]
    fn disconnected_is_down_not_healing() {
        let s = Snapshot::disconnected("127.0.0.1:38332");
        assert_eq!(s.health, "dead");
        assert_eq!(s.health_label, "Node down");
        assert_eq!(s.node_status, "Down");
        assert!(!s.connected);
        assert!(!s.node_running);
        assert_eq!(s.node_power_label, "Turn Node On");
    }

    #[test]
    fn first_sync_is_ibd_not_healing() {
        assert!(is_ibd(true, 0, 322_549, 322_549, false));
        assert!(is_ibd(false, 2_000, 322_549, 320_549, false));
        assert!(is_ibd(false, 300_000, 322_549, 22_549, false));
    }

    #[test]
    fn catch_up_after_tip_is_healing_not_ibd() {
        assert!(!is_ibd(false, 322_540, 322_549, 9, true));
        assert!(!is_ibd(false, 322_549, 322_549, 0, true));
    }

    #[test]
    fn restart_near_tip_is_healing() {
        assert!(!is_ibd(false, 322_540, 322_549, 9, false));
        assert!(is_ibd(false, 322_000, 322_549, 549, false));
    }

    #[test]
    fn switching_rpc_keeps_each_chain_feed() {
        let mut g = LiveState::new("127.0.0.1:38332".into());
        {
            let c = chain_mut(&mut g);
            c.last = Some(100);
            c.feed.push_front(FeedItem::block(100));
        }
        g.rpc_addr = "127.0.0.1:48332".into();
        {
            let c = chain_mut(&mut g);
            c.last = Some(50);
            c.feed.push_front(FeedItem::block(50));
        }
        g.rpc_addr = "127.0.0.1:38332".into();
        let signet = chain_mut(&mut g);
        assert_eq!(signet.last, Some(100));
        assert_eq!(signet.feed.front().unwrap().label, "100");
        g.rpc_addr = "127.0.0.1:48332".into();
        let testnet = chain_mut(&mut g);
        assert_eq!(testnet.last, Some(50));
        assert_eq!(testnet.feed.front().unwrap().label, "50");
    }

    #[test]
    fn height_rewind_clears_synced_and_arrival() {
        let mut g = LiveState::new("127.0.0.1:48332".into());
        {
            let c = chain_mut(&mut g);
            c.last = Some(152_885);
            c.feed.push_front(FeedItem::block(152_885));
            c.ever_synced = true;
            c.bucket_delta = 40;
            c.arrival = VecDeque::from(vec![9; HIST_BARS]);
        }
        {
            let c = chain_mut(&mut g);
            rewind_chain_if_needed(c, 0);
            crate::feed::advance_feed(&mut c.feed, &mut c.last, 0);
            assert!(!c.ever_synced);
            assert_eq!(c.bucket_delta, 0);
            assert!(c.arrival.iter().all(|&n| n == 0));
            assert_eq!(c.feed.len(), 1);
            assert_eq!(c.feed[0], FeedItem::block(0));
        }
    }

    #[test]
    fn unreachable_keeps_last_chain_status() {
        let mut g = LiveState::new("127.0.0.1:38332".into());
        g.snapshot.connected = true;
        g.snapshot.local_height = 184_800;
        g.snapshot.network_height = 322_555;
        g.snapshot.network = "signet".into();
        g.snapshot.sync_pct_num = "57.3".into();
        g.snapshot.network_active = true;
        g.snapshot.ibd = true;
        chain_mut(&mut g).last_snapshot = Some(g.snapshot.clone());
        mark_unreachable(&mut g, "RPC 127.0.0.1:38332 timed out".into());
        assert!(!g.snapshot.connected);
        assert_eq!(g.snapshot.health, "heal");
        assert_eq!(g.snapshot.local_height, 184_800);
        assert_eq!(g.snapshot.network, "signet");
        assert_eq!(g.snapshot.sync_pct_num, "57.3");
        assert!(g.snapshot.frozen);
        assert_eq!(g.snapshot.health_label, "Frozen");
        assert!(g.snapshot.network_active);
    }

    #[test]
    fn connection_refused_is_down_not_frozen() {
        let mut g = LiveState::new("127.0.0.1:38332".into());
        mark_unreachable(&mut g, "connect 127.0.0.1:38332".into());
        assert!(!g.snapshot.frozen);
        assert_eq!(g.snapshot.health, "dead");
        assert_eq!(g.snapshot.health_label, "Node down");
        assert_eq!(g.snapshot.network, "signet");
    }
}
