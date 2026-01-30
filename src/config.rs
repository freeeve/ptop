use clap::Parser;
use std::net::IpAddr;
use std::process::Command;

/// Network latency monitor - htop for ping
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    /// Target hosts to ping (IP addresses or hostnames)
    #[arg(short, long)]
    pub targets: Vec<String>,

    /// Ping interval in milliseconds
    #[arg(short, long, default_value = "1000")]
    pub interval: u64,

    /// Include default targets (1.1.1.1, 8.8.8.8, 9.9.9.9)
    #[arg(short = 'd', long, default_value = "true")]
    pub defaults: bool,

    /// Skip auto-detection of local gateway
    #[arg(long, default_value = "false")]
    pub no_gateway: bool,

    /// Log raw ping data for replay (saves to ~/.ptop/logs/)
    #[arg(short = 'l', long)]
    pub log_raw: bool,

    /// Save session summary on exit (saves to ~/.ptop/sessions/)
    #[arg(short = 's', long)]
    pub summary: bool,

    /// Replay a previously recorded session
    #[arg(long, value_name = "PATH")]
    pub replay: Option<String>,

    /// Replay speed multiplier (default: 1.0)
    #[arg(long, default_value = "1.0")]
    pub speed: f64,

    /// List available log files for replay
    #[arg(long)]
    pub list_logs: bool,

    /// List available session summaries
    #[arg(long)]
    pub list_sessions: bool,
}

#[derive(Debug, Clone)]
pub struct Target {
    pub name: String,
    pub addr: IpAddr,
}

impl Target {
    pub fn new(name: impl Into<String>, addr: IpAddr) -> Self {
        Self {
            name: name.into(),
            addr,
        }
    }
}

/// Returns the default ping targets.
pub fn default_targets() -> Vec<Target> {
    vec![
        Target::new("Cloudflare", "1.1.1.1".parse().unwrap()),
        Target::new("Google", "8.8.8.8".parse().unwrap()),
        Target::new("Quad9", "9.9.9.9".parse().unwrap()),
    ]
}

/// Attempts to detect the local gateway IP.
pub fn detect_gateway() -> Option<Target> {
    // Try macOS/BSD style first
    if let Some(gw) = detect_gateway_macos() {
        return Some(gw);
    }

    // Try Linux style
    if let Some(gw) = detect_gateway_linux() {
        return Some(gw);
    }

    None
}

fn detect_gateway_macos() -> Option<Target> {
    let output = Command::new("route")
        .args(["-n", "get", "default"])
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.starts_with("gateway:") {
            let ip_str = line.strip_prefix("gateway:")?.trim();
            if let Ok(addr) = ip_str.parse::<IpAddr>() {
                return Some(Target::new("Gateway", addr));
            }
        }
    }
    None
}

fn detect_gateway_linux() -> Option<Target> {
    let output = Command::new("ip")
        .args(["route", "show", "default"])
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Format: "default via 192.168.1.1 dev eth0 ..."
    let parts: Vec<&str> = stdout.split_whitespace().collect();
    if parts.len() >= 3
        && parts[0] == "default"
        && parts[1] == "via"
        && let Ok(addr) = parts[2].parse::<IpAddr>()
    {
        return Some(Target::new("Gateway", addr));
    }
    None
}

/// Builds the complete target list based on CLI args.
pub fn build_target_list(args: &Args) -> Vec<Target> {
    let mut targets = Vec::new();

    // Add gateway first if not disabled
    if !args.no_gateway
        && let Some(gw) = detect_gateway()
    {
        targets.push(gw);
    }

    // Add default targets
    if args.defaults {
        targets.extend(default_targets());
    }

    // Add user-specified targets
    for t in &args.targets {
        if let Ok(addr) = t.parse::<IpAddr>() {
            targets.push(Target::new(t.clone(), addr));
        } else {
            // Try to resolve hostname
            if let Ok(addrs) = std::net::ToSocketAddrs::to_socket_addrs(&(t.as_str(), 0))
                && let Some(sock_addr) = addrs.into_iter().next()
            {
                targets.push(Target::new(t.clone(), sock_addr.ip()));
            }
        }
    }

    targets
}
