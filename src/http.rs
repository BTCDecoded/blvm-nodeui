use crate::state::{poll_once, stamp_power, Shared};
use crate::node_ctl::{self, PowerPhase};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;

const INDEX: &str = include_str!("../static/index.html");
const CSS: &str = include_str!("../static/app.css");
const JS: &str = include_str!("../static/app.js");
const LOGO: &[u8] = include_bytes!("../static/logo.png");
const ROBOTO_400: &[u8] = include_bytes!("../static/fonts/roboto-400.woff2");
const ROBOTO_500: &[u8] = include_bytes!("../static/fonts/roboto-500.woff2");
const ROBOTO_700: &[u8] = include_bytes!("../static/fonts/roboto-700.woff2");

pub async fn serve(addr: SocketAddr, state: Shared) -> anyhow::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("blvm-ui listening on http://{addr}");
    loop {
        let (stream, _) = listener.accept().await?;
        let io = TokioIo::new(stream);
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let svc = service_fn(move |req| {
                let state = Arc::clone(&state);
                async move { handle(req, state).await }
            });
            if let Err(e) = http1::Builder::new().serve_connection(io, svc).await {
                tracing::debug!("http conn: {e}");
            }
        });
    }
}

async fn handle(
    req: Request<Incoming>,
    state: Shared,
) -> Result<Response<Full<Bytes>>, Infallible> {
    let path = req.uri().path().to_string();
    let method = req.method().clone();
    let resp = match (method, path.as_str()) {
        (Method::GET, "/") | (Method::GET, "/index.html") => html(INDEX),
        (Method::GET, "/app.css") => css(CSS),
        (Method::GET, "/app.js") => js(JS),
        (Method::GET, "/logo.png") | (Method::GET, "/favicon.png") => png(LOGO),
        (Method::GET, "/fonts/roboto-400.woff2") => font(ROBOTO_400),
        (Method::GET, "/fonts/roboto-500.woff2") => font(ROBOTO_500),
        (Method::GET, "/fonts/roboto-700.woff2") => font(ROBOTO_700),
        (Method::GET, "/api/status") => {
            let snap = state.read().await.snapshot.clone();
            json_ok(&snap)
        }
        (Method::POST, "/api/node") => power_node(req, state).await,
        (Method::POST, "/api/connect") => {
            let collected = req.into_body().collect().await.ok().map(|c| c.to_bytes());
            let addr = collected
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .and_then(|v| v.get("rpc").and_then(|s| s.as_str()).map(|s| s.to_string()))
                .unwrap_or_else(|| "127.0.0.1:48332".into());
            {
                let mut g = state.write().await;
                g.switch_rpc(addr);
            }
            let state_bg = Arc::clone(&state);
            tokio::spawn(async move {
                poll_once(&state_bg).await;
            });
            let snap = state.read().await.snapshot.clone();
            json_ok(&snap)
        }
        (Method::POST, "/api/rpc") => {
            let collected = req.into_body().collect().await.ok().map(|c| c.to_bytes());
            let body = collected
                .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                .unwrap_or(serde_json::json!({}));
            let method = body
                .get("method")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string();
            let params = body.get("params").cloned().unwrap_or(serde_json::json!([]));
            if !crate::rpc::allowed_setting(&method) {
                let mut r = json_ok(&serde_json::json!({
                    "ok": false,
                    "error": "method not allowed"
                }));
                *r.status_mut() = StatusCode::BAD_REQUEST;
                r
            } else {
                let rpc_addr = { state.read().await.rpc_addr.clone() };
                match crate::rpc::call_params(&rpc_addr, &method, params).await {
                    Ok(result) => {
                        let state_bg = Arc::clone(&state);
                        tokio::spawn(async move {
                            poll_once(&state_bg).await;
                        });
                        json_ok(&serde_json::json!({ "ok": true, "result": result }))
                    }
                    Err(e) => json_ok(&serde_json::json!({
                        "ok": false,
                        "error": e.to_string()
                    })),
                }
            }
        }
        _ => {
            let mut r = Response::new(Full::new(Bytes::from("not found")));
            *r.status_mut() = StatusCode::NOT_FOUND;
            r
        }
    };
    Ok(resp)
}

async fn power_node(
    req: Request<Incoming>,
    state: Shared,
) -> Response<Full<Bytes>> {
    let lock = { state.read().await.power_lock.clone() };
    let _busy = lock.lock().await;
    let collected = req.into_body().collect().await.ok().map(|c| c.to_bytes());
    let action = collected
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| {
            v.get("action")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "toggle".into());
    let rpc = { state.read().await.rpc_addr.clone() };
    let going_on = match action.as_str() {
        "on" => true,
        "off" => false,
        _ => !node_ctl::node_is_up(&rpc).await,
    };
    {
        let mut g = state.write().await;
        g.power_phase = if going_on {
            PowerPhase::Starting
        } else {
            PowerPhase::Stopping
        };
        stamp_power(&mut g);
    }
    let mut launch = { state.read().await.launch.clone() };
    let result = if going_on {
        node_ctl::start(&rpc, &mut launch).await
    } else {
        node_ctl::stop(&rpc).await
    };
    {
        let mut g = state.write().await;
        g.power_phase = PowerPhase::Idle;
        g.launch = launch;
        stamp_power(&mut g);
    }
    poll_once(&state).await;
    let snap = { state.read().await.snapshot.clone() };
    json_ok(&serde_json::json!({
        "ok": result.ok,
        "message": result.message,
        "error": result.error,
        "status": snap
    }))
}

fn html(s: &'static str) -> Response<Full<Bytes>> {
    typed(s, "text/html; charset=utf-8")
}
fn css(s: &'static str) -> Response<Full<Bytes>> {
    typed(s, "text/css; charset=utf-8")
}
fn js(s: &'static str) -> Response<Full<Bytes>> {
    typed(s, "text/javascript; charset=utf-8")
}
fn png(b: &'static [u8]) -> Response<Full<Bytes>> {
    let mut r = Response::new(Full::new(Bytes::from_static(b)));
    r.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("image/png"),
    );
    r
}
fn font(b: &'static [u8]) -> Response<Full<Bytes>> {
    let mut r = Response::new(Full::new(Bytes::from_static(b)));
    r.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("font/woff2"),
    );
    r.headers_mut().insert(
        hyper::header::CACHE_CONTROL,
        hyper::header::HeaderValue::from_static("public, max-age=31536000"),
    );
    r
}

fn typed(s: &'static str, ct: &'static str) -> Response<Full<Bytes>> {
    let mut r = Response::new(Full::new(Bytes::from_static(s.as_bytes())));
    r.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static(ct),
    );
    r.headers_mut().insert(
        hyper::header::CACHE_CONTROL,
        hyper::header::HeaderValue::from_static("no-store"),
    );
    r
}

fn json_ok<T: serde::Serialize>(v: &T) -> Response<Full<Bytes>> {
    let body = serde_json::to_vec(v).unwrap_or_else(|_| b"{}".to_vec());
    let mut r = Response::new(Full::new(Bytes::from(body)));
    r.headers_mut().insert(
        hyper::header::CONTENT_TYPE,
        hyper::header::HeaderValue::from_static("application/json"),
    );
    r
}

