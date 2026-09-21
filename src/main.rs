use anyhow::Result;
use blvm_ui::state::{poll_once, LiveState, Shared};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::time::{interval, Duration};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "blvm_ui=info".into()),
        )
        .init();

    let listen: SocketAddr = std::env::var("BLVM_UI_LISTEN")
        .unwrap_or_else(|_| "127.0.0.1:3847".into())
        .parse()?;
    let rpc = std::env::var("BLVM_UI_RPC").unwrap_or_else(|_| "127.0.0.1:48332".into());

    let state: Shared = Arc::new(RwLock::new(LiveState::new(rpc)));
    {
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            // Node caps new RPC sockets at 10 per IP per 60s; 6s + one batch POST is the max rate.
            let mut tick = interval(Duration::from_secs(6));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                poll_once(&state).await;
            }
        });
    }

    blvm_ui::http::serve(listen, state).await?;
    Ok(())
}
