use crate::config::Target;
use crate::stats::PingResult;
use anyhow::Result;
use std::net::IpAddr;
use std::time::Duration;
use surge_ping::{Client, Config, ICMP, PingIdentifier, PingSequence};
use tokio::sync::mpsc;
use tokio::time::{MissedTickBehavior, interval};

/// Default ping timeout.
const PING_TIMEOUT: Duration = Duration::from_secs(4);

/// Payload size for ICMP packets.
const PAYLOAD_SIZE: usize = 56;

/// Message sent from pinger to main app.
#[derive(Debug)]
pub struct PingUpdate {
    pub target_idx: usize,
    pub result: PingResult,
}

/// Creates the appropriate ICMP client based on IP version.
pub async fn create_client_v4() -> Result<Client> {
    let config = Config::default();
    let client = Client::new(&config)?;
    Ok(client)
}

pub async fn create_client_v6() -> Result<Client> {
    let config = Config::builder().kind(ICMP::V6).build();
    let client = Client::new(&config)?;
    Ok(client)
}

/// Creates an ICMP client for the given address type.
async fn create_client(addr: IpAddr) -> Result<Client> {
    match addr {
        IpAddr::V4(_) => create_client_v4().await,
        IpAddr::V6(_) => create_client_v6().await,
    }
}

/// Checks if an error indicates a stale socket that needs recreation.
fn is_network_error(err: &str) -> bool {
    let err_lower = err.to_lowercase();
    err_lower.contains("network is unreachable")
        || err_lower.contains("no route to host")
        || err_lower.contains("network unreachable")
        || err_lower.contains("host unreachable")
        || err_lower.contains("invalid argument")
        || err_lower.contains("bad file descriptor")
        || err_lower.contains("socket")
}

/// Spawns a pinger task for a target.
pub fn spawn_pinger(
    target_idx: usize,
    target: Target,
    ping_interval: Duration,
    tx: mpsc::UnboundedSender<PingUpdate>,
) {
    tokio::spawn(async move {
        let payload = vec![0u8; PAYLOAD_SIZE];
        let mut seq = 0u16;
        let mut consecutive_errors = 0u32;

        // Use interval with skip behavior to handle slow pings gracefully
        let mut tick = interval(ping_interval);
        tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

        let mut client: Option<Client> = None;

        loop {
            tick.tick().await;

            // Create or recreate client if needed
            if client.is_none() {
                match create_client(target.addr).await {
                    Ok(c) => {
                        client = Some(c);
                        consecutive_errors = 0;
                    }
                    Err(e) => {
                        let _ = tx.send(PingUpdate {
                            target_idx,
                            result: PingResult::Error(format!("Client error: {}", e)),
                        });
                        // Wait before retrying client creation
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                }
            }

            let c = client.as_ref().unwrap();
            let mut pinger = c.pinger(target.addr, PingIdentifier(rand::random())).await;
            pinger.timeout(PING_TIMEOUT);

            let result = match pinger.ping(PingSequence(seq), &payload).await {
                Ok((_, duration)) => {
                    consecutive_errors = 0;
                    PingResult::Success(duration)
                }
                Err(e) => {
                    let err_str = e.to_string();
                    if err_str.contains("timeout") {
                        consecutive_errors = 0;
                        PingResult::Timeout
                    } else {
                        consecutive_errors += 1;
                        // Recreate client after consecutive network errors
                        if is_network_error(&err_str) && consecutive_errors >= 3 {
                            client = None;
                        }
                        PingResult::Error(err_str)
                    }
                }
            };

            if tx.send(PingUpdate { target_idx, result }).is_err() {
                // Channel closed, exit task
                break;
            }

            seq = seq.wrapping_add(1);
        }
    });
}
