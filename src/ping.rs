use crate::config::Target;
use crate::stats::PingResult;
use anyhow::Result;
use std::net::IpAddr;
use std::time::{Duration, Instant};
use surge_ping::{Client, Config, ICMP, PingIdentifier, PingSequence};
use tokio::sync::mpsc;

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

/// Spawns a pinger task for a target.
pub fn spawn_pinger(
    target_idx: usize,
    target: Target,
    interval: Duration,
    tx: mpsc::UnboundedSender<PingUpdate>,
) {
    tokio::spawn(async move {
        let client_result = match target.addr {
            IpAddr::V4(_) => create_client_v4().await,
            IpAddr::V6(_) => create_client_v6().await,
        };

        let client = match client_result {
            Ok(c) => c,
            Err(e) => {
                let _ = tx.send(PingUpdate {
                    target_idx,
                    result: PingResult::Error(format!("Failed to create client: {}", e)),
                });
                return;
            }
        };

        let payload = vec![0u8; PAYLOAD_SIZE];
        let mut seq = 0u16;

        loop {
            let cycle_start = Instant::now();

            let mut pinger = client
                .pinger(target.addr, PingIdentifier(rand::random()))
                .await;
            pinger.timeout(PING_TIMEOUT);

            let result = match pinger.ping(PingSequence(seq), &payload).await {
                Ok((_, duration)) => PingResult::Success(duration),
                Err(e) => {
                    if e.to_string().contains("timeout") {
                        PingResult::Timeout
                    } else {
                        PingResult::Error(e.to_string())
                    }
                }
            };

            if tx.send(PingUpdate { target_idx, result }).is_err() {
                // Channel closed, exit task
                break;
            }

            seq = seq.wrapping_add(1);

            // Sleep for remaining time in the interval (accounting for ping duration)
            let elapsed = cycle_start.elapsed();
            if elapsed < interval {
                tokio::time::sleep(interval - elapsed).await;
            }
        }
    });
}
