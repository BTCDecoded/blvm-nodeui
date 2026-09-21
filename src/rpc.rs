//! Tiny JSON-RPC/2.0 client over HTTP (BLVM RPC). No extra HTTP client crate.
//!
//! The node rate-limits *new* TCP connections per IP (10 per 60s) and closes
//! each HTTP response. Polls therefore use one batch POST, not one socket per method.

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub async fn call(rpc_addr: &str, method: &str) -> Result<Value> {
    call_params(rpc_addr, method, json!([])).await
}

pub async fn call_params(rpc_addr: &str, method: &str, params: Value) -> Result<Value> {
    let payload = json!({
        "jsonrpc": "2.0",
        "id": "blvm-ui",
        "method": method,
        "params": params
    });
    let v = post(rpc_addr, &payload).await?;
    result_of(&v).ok_or_else(|| anyhow!("RPC {method}: {}", v.get("error").unwrap_or(&Value::Null)))
}

/// One TCP round-trip for several methods. Index matches `methods`.
pub async fn call_batch(rpc_addr: &str, methods: &[&str]) -> Result<Vec<Option<Value>>> {
    let payload = Value::Array(
        methods
            .iter()
            .enumerate()
            .map(|(i, method)| {
                json!({
                    "jsonrpc": "2.0",
                    "id": i,
                    "method": method,
                    "params": []
                })
            })
            .collect(),
    );
    let v = post(rpc_addr, &payload).await?;
    let arr = v
        .as_array()
        .ok_or_else(|| anyhow!("RPC batch response was not an array"))?;
    let mut out = vec![None; methods.len()];
    for item in arr {
        let id = item.get("id").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
        if id < out.len() {
            out[id] = result_of(item);
        }
    }
    Ok(out)
}

fn result_of(v: &Value) -> Option<Value> {
    if v.get("error").is_some_and(|e| !e.is_null()) {
        None
    } else {
        Some(v.get("result").cloned().unwrap_or(Value::Null))
    }
}

async fn post(rpc_addr: &str, payload: &Value) -> Result<Value> {
    let stream = match tokio::time::timeout(Duration::from_millis(800), TcpStream::connect(rpc_addr)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => return Err(e).with_context(|| format!("connect {rpc_addr}")),
        Err(_) => return Err(anyhow!("RPC {rpc_addr} frozen")),
    };
    tokio::time::timeout(Duration::from_secs(2), rpc_exchange(stream, rpc_addr, payload))
        .await
        .map_err(|_| anyhow!("RPC {rpc_addr} frozen"))?
}

async fn rpc_exchange(mut stream: TcpStream, rpc_addr: &str, payload: &Value) -> Result<Value> {
    let body = payload.to_string();
    let req = format!(
        "POST / HTTP/1.1\r\nHost: {rpc_addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).await?;
    let mut buf = Vec::with_capacity(8192);
    stream.read_to_end(&mut buf).await?;
    let text = String::from_utf8_lossy(&buf);
    let resp_body = text
        .split("\r\n\r\n")
        .nth(1)
        .ok_or_else(|| anyhow!("RPC response had no body"))?;
    serde_json::from_str(resp_body.trim()).context("RPC JSON")
}

/// TCP accepted but the node did not answer (IBD stuck, lock held, etc.).
pub fn is_frozen_error(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("frozen") || e.contains("timed out")
}

/// Node RPCs the Settings page is allowed to invoke.
pub fn allowed_setting(method: &str) -> bool {
    matches!(
        method,
        "setban" | "listbanned" | "clearbanned" | "disconnectnode" | "addnode" | "setnetworkactive"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_whitelist_is_peer_and_network_only() {
        assert!(allowed_setting("addnode"));
        assert!(allowed_setting("disconnectnode"));
        assert!(allowed_setting("setban"));
        assert!(allowed_setting("listbanned"));
        assert!(allowed_setting("clearbanned"));
        assert!(allowed_setting("setnetworkactive"));
        assert!(!allowed_setting("getblockchaininfo"));
        assert!(!allowed_setting("stop"));
        assert!(!allowed_setting(""));
    }

    #[test]
    fn batch_result_skips_errors() {
        let ok = json!({"jsonrpc":"2.0","id":0,"result":{"blocks":1}});
        let err = json!({"jsonrpc":"2.0","id":1,"error":{"code":-1,"message":"no"}});
        assert_eq!(result_of(&ok), Some(json!({"blocks":1})));
        assert_eq!(result_of(&err), None);
    }

    #[test]
    fn frozen_vs_connect_errors() {
        assert!(is_frozen_error("RPC 127.0.0.1:38332 frozen"));
        assert!(is_frozen_error("RPC 127.0.0.1:38332 timed out"));
        assert!(!is_frozen_error("connect 127.0.0.1:38332"));
        assert!(!is_frozen_error("Connection refused"));
    }
}
