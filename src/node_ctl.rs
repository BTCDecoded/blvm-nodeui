//! Start / stop the node process from the console. The UI process is never killed here.

use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::sleep;

const TERM_WAIT: Duration = Duration::from_secs(20);
const KILL_WAIT: Duration = Duration::from_secs(3);
const START_WAIT: Duration = Duration::from_secs(45);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerPhase {
    Idle,
    Stopping,
    Starting,
}

impl Default for PowerPhase {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Clone)]
pub struct LaunchSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub cwd: PathBuf,
}

#[derive(Debug, Clone, Serialize)]
pub struct PowerResult {
    pub ok: bool,
    pub message: String,
    pub error: Option<String>,
}

pub async fn toggle(rpc_addr: &str, launch: &mut Option<LaunchSpec>) -> PowerResult {
    if node_is_up(rpc_addr).await {
        stop(rpc_addr).await
    } else {
        start(rpc_addr, launch).await
    }
}

pub async fn node_is_up(rpc_addr: &str) -> bool {
    // lsof only. Do not open a new RPC TCP here — the node allows 10 new
    // connections per IP per 60s, and the console poll already uses that budget.
    listener_pid(rpc_addr).is_some()
}

pub async fn stop(rpc_addr: &str) -> PowerResult {
    let self_pid = std::process::id();
    let pid = listener_pid(rpc_addr).filter(|p| *p != self_pid && is_blvm_node(*p));

    let Some(pid) = pid else {
        return PowerResult {
            ok: true,
            message: "Node is already off.".into(),
            error: None,
        };
    };

    // SIGTERM is this node's graceful path (flush UTXO / drain). RPC `stop` needs
    // a new TCP socket; the console poll already sits on the 10/60s cap, so that
    // call gets RST (`Connection rate limit exceeded`) and is not a shutdown.
    tracing::info!(pid, rpc_addr, "node power: SIGTERM (graceful)");
    send_signal(pid, libc::SIGTERM);
    if wait_until_down(rpc_addr, TERM_WAIT).await {
        return PowerResult {
            ok: true,
            message: "Node stopped (SIGTERM).".into(),
            error: None,
        };
    }

    tracing::warn!(pid, "node power: SIGKILL after SIGTERM did not finish");
    send_signal(pid, libc::SIGKILL);
    if wait_until_down(rpc_addr, KILL_WAIT).await {
        return PowerResult {
            ok: true,
            message: "Node was killed after a hung shutdown.".into(),
            error: None,
        };
    }
    PowerResult {
        ok: false,
        message: String::new(),
        error: Some("Node did not exit after SIGKILL.".into()),
    }
}

pub async fn start(rpc_addr: &str, launch: &mut Option<LaunchSpec>) -> PowerResult {
    if node_is_up(rpc_addr).await {
        if launch.is_none() {
            if let Some(pid) = listener_pid(rpc_addr) {
                *launch = capture_launch(pid);
            }
        }
        return PowerResult {
            ok: true,
            message: "Node is already on.".into(),
            error: None,
        };
    }

    let spec = match launch.clone().or_else(|| infer_launch(rpc_addr).ok()) {
        Some(s) => s,
        None => {
            return PowerResult {
                ok: false,
                message: String::new(),
                error: Some(
                    "No blvm binary found. Set BLVM_UI_NODE_BIN or start the node once from a terminal so the console can remember the command.".into(),
                ),
            };
        }
    };

    tracing::info!(
        program = %spec.program.display(),
        cwd = %spec.cwd.display(),
        "node power: starting"
    );

    if let Err(e) = spawn_detached(&spec) {
        return PowerResult {
            ok: false,
            message: String::new(),
            error: Some(format!("Failed to start node: {e}")),
        };
    }

    let deadline = tokio::time::Instant::now() + START_WAIT;
    while tokio::time::Instant::now() < deadline {
        if node_is_up(rpc_addr).await {
            if let Some(pid) = listener_pid(rpc_addr) {
                if let Some(captured) = capture_launch(pid) {
                    *launch = Some(captured);
                } else {
                    *launch = Some(spec);
                }
            }
            return PowerResult {
                ok: true,
                message: "Node started.".into(),
                error: None,
            };
        }
        sleep(Duration::from_millis(400)).await;
    }

    PowerResult {
        ok: false,
        message: String::new(),
        error: Some(format!(
            "Started {} but RPC {} did not come up in {}s.",
            spec.program.display(),
            rpc_addr,
            START_WAIT.as_secs()
        )),
    }
}

pub fn remember_if_running(rpc_addr: &str, launch: &mut Option<LaunchSpec>) {
    if launch.is_some() {
        return;
    }
    if let Some(pid) = listener_pid(rpc_addr) {
        *launch = capture_launch(pid);
    }
}

pub fn rpc_port(rpc_addr: &str) -> Option<u16> {
    rpc_addr
        .rsplit_once(':')
        .and_then(|(_, p)| p.parse().ok())
}

fn listener_pid(rpc_addr: &str) -> Option<u32> {
    let port = rpc_port(rpc_addr)?;
    let out = std::process::Command::new("lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN", "-t"])
        .output()
        .ok()?;
    if !out.status.success() && out.stdout.is_empty() {
        return None;
    }
    let self_pid = std::process::id();
    parse_pids(&String::from_utf8_lossy(&out.stdout))
        .into_iter()
        .find(|p| *p != self_pid && is_blvm_node(*p))
}

fn parse_pids(s: &str) -> Vec<u32> {
    s.split_whitespace().filter_map(|w| w.parse().ok()).collect()
}

fn is_blvm_node(pid: u32) -> bool {
    if pid == std::process::id() {
        return false;
    }
    let name = process_comm(pid).unwrap_or_default();
    let lower = name.to_ascii_lowercase();
    if lower.contains("blvm-ui") {
        return false;
    }
    lower.contains("blvm")
}

fn process_comm(pid: u32) -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn capture_launch(pid: u32) -> Option<LaunchSpec> {
    if !is_blvm_node(pid) {
        return None;
    }
    let out = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-www", "-o", "args="])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&out.stdout);
    let argv = shell_split(line.trim());
    if argv.is_empty() {
        return None;
    }
    let program = PathBuf::from(&argv[0]);
    let args = argv[1..].to_vec();
    let cwd = process_cwd(pid).unwrap_or_else(|| {
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
    });
    Some(LaunchSpec { program, args, cwd })
}

fn process_cwd(pid: u32) -> Option<PathBuf> {
    let out = std::process::Command::new("lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
        .output()
        .ok()?;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(rest) = line.strip_prefix('n') {
            let p = PathBuf::from(rest);
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    None
}

/// Split a `ps` args line. Quoted tokens are not used in our start commands.
fn shell_split(s: &str) -> Vec<String> {
    s.split_whitespace().map(|t| t.to_string()).collect()
}

pub fn infer_launch(rpc_addr: &str) -> Result<LaunchSpec> {
    let port = rpc_port(rpc_addr).ok_or_else(|| anyhow!("bad RPC address {rpc_addr}"))?;
    let (network, data_dir, p2p) = match port {
        48332 => ("testnet4", "data-testnet4", 48333u16),
        38332 => ("signet", "data-signet", 38333),
        18443 => ("regtest", "data", 18444),
        18332 => ("testnet", "data-testnet", 18333),
        8332 => ("mainnet", "data-mainnet", 8333),
        other => {
            return Err(anyhow!(
                "do not know how to start a node for RPC port {other}; start it once from a terminal"
            ));
        }
    };
    let program = find_blvm_bin().ok_or_else(|| anyhow!("blvm binary not found"))?;
    let cwd = find_node_cwd(data_dir);
    let rpc = if rpc_addr.contains(':') {
        rpc_addr.to_string()
    } else {
        format!("127.0.0.1:{port}")
    };
    Ok(LaunchSpec {
        program,
        args: vec![
            "--network".into(),
            network.into(),
            "--data-dir".into(),
            data_dir.into(),
            "--listen-addr".into(),
            format!("0.0.0.0:{p2p}"),
            "--rpc-addr".into(),
            rpc,
        ],
        cwd,
    })
}

fn find_blvm_bin() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("BLVM_UI_NODE_BIN") {
        let path = PathBuf::from(p);
        if is_executable(&path) {
            return Some(path);
        }
    }
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("blvm"));
            if let Some(target) = dir.parent() {
                candidates.push(target.join("release-fast").join("blvm"));
                candidates.push(target.join("release").join("blvm"));
                candidates.push(target.join("debug").join("blvm"));
            }
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        for rel in [
            "target/release-fast/blvm",
            "target/release/blvm",
            "target/debug/blvm",
            "blvm/target/release-fast/blvm",
            "blvm/target/release/blvm",
            "blvm/target/debug/blvm",
        ] {
            candidates.push(cwd.join(rel));
        }
    }
    candidates.into_iter().find(|p| is_executable(p))
}

fn find_node_cwd(data_dir: &str) -> PathBuf {
    if let Ok(p) = std::env::var("BLVM_UI_NODE_CWD") {
        return PathBuf::from(p);
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if cwd.join(data_dir).is_dir() {
        return cwd;
    }
    if cwd.join("blvm").join(data_dir).is_dir() {
        return cwd.join("blvm");
    }
    if cwd.file_name().is_some_and(|n| n == "blvm") {
        return cwd;
    }
    cwd
}

fn is_executable(path: &Path) -> bool {
    path.is_file()
}

fn spawn_detached(spec: &LaunchSpec) -> Result<()> {
    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args)
        .current_dir(&spec.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .kill_on_drop(false);
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }
    let child = cmd
        .spawn()
        .with_context(|| format!("spawn {}", spec.program.display()))?;
    // Detached: do not wait, do not kill when this handle drops.
    std::mem::forget(child);
    Ok(())
}

async fn wait_until_down(rpc_addr: &str, budget: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + budget;
    while tokio::time::Instant::now() < deadline {
        if !node_is_up(rpc_addr).await {
            return true;
        }
        sleep(Duration::from_millis(250)).await;
    }
    !node_is_up(rpc_addr).await
}

#[cfg(unix)]
fn send_signal(pid: u32, sig: i32) {
    unsafe {
        libc::kill(pid as i32, sig);
    }
}

#[cfg(not(unix))]
fn send_signal(_pid: u32, _sig: i32) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_port_from_addr() {
        assert_eq!(rpc_port("127.0.0.1:48332"), Some(48332));
        assert_eq!(rpc_port("127.0.0.1:38332"), Some(38332));
        assert!(rpc_port("nope").is_none());
    }

    #[test]
    fn parse_lsof_pid_lines() {
        assert_eq!(parse_pids("7777\n"), vec![7777]);
        assert_eq!(parse_pids("11\n22\n"), vec![11, 22]);
        assert!(parse_pids("").is_empty());
    }

    #[test]
    fn self_pid_is_not_a_node() {
        assert!(!is_blvm_node(std::process::id()));
    }

    #[test]
    fn infer_testnet4_args() {
        let spec = infer_launch("127.0.0.1:48332");
        if let Ok(spec) = spec {
            assert!(spec.args.windows(2).any(|w| w[0] == "--network" && w[1] == "testnet4"));
            assert!(spec.args.windows(2).any(|w| w[0] == "--rpc-addr" && w[1] == "127.0.0.1:48332"));
        }
    }

    #[test]
    fn infer_unknown_port_errors() {
        let err = infer_launch("127.0.0.1:9999").unwrap_err().to_string();
        assert!(err.contains("9999"));
    }

    #[test]
    fn shell_split_keeps_flags() {
        let v = shell_split("./target/release-fast/blvm --network testnet4 --data-dir ./data-testnet4");
        assert_eq!(v[1], "--network");
        assert_eq!(v[2], "testnet4");
    }
}
