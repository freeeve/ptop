# ptop

A network latency monitor with a terminal UI - like htop, but for ping.

![ptop demo](readme/demo.gif)

## Features

- **Real-time monitoring** - Ping multiple targets concurrently with live updates
- **Rich statistics** - Min, max, average, P50, P95, jitter, packet loss
- **Quality metrics** - MOS score and letter grades (A-F) based on VoIP standards
- **Visual history** - Sparkline charts showing latency over time
- **Detail view** - Histogram, percentile breakdown, loss streaks per target
- **Session logging** - Record sessions for later replay and analysis
- **Replay mode** - Play back recorded sessions at adjustable speeds

## Installation

### Homebrew (macOS/Linux)

```bash
brew tap freeeve/tap
brew install ptop
```

### Download binary

Download the latest release for your platform from the [releases page](https://github.com/freeeve/ptop/releases).

```bash
# macOS (Apple Silicon)
curl -L https://github.com/freeeve/ptop/releases/latest/download/ptop-macos-aarch64.tar.gz | tar xz
sudo mv ptop /usr/local/bin/

# macOS (Intel)
curl -L https://github.com/freeeve/ptop/releases/latest/download/ptop-macos-x86_64.tar.gz | tar xz
sudo mv ptop /usr/local/bin/

# Linux (x86_64)
curl -L https://github.com/freeeve/ptop/releases/latest/download/ptop-linux-x86_64.tar.gz | tar xz
sudo mv ptop /usr/local/bin/

# Linux (ARM64)
curl -L https://github.com/freeeve/ptop/releases/latest/download/ptop-linux-aarch64.tar.gz | tar xz
sudo mv ptop /usr/local/bin/
```

### From source

```bash
git clone https://github.com/freeeve/ptop
cd ptop
cargo build --release
sudo cp target/release/ptop /usr/local/bin/
```

### Cargo

```bash
cargo install ptop
```

### Set capabilities (Linux, optional)

To run without sudo on Linux:

```bash
sudo setcap cap_net_raw=ep /usr/local/bin/ptop
```

## Usage

ptop requires elevated privileges to send ICMP packets.

```bash
# Basic usage with default targets (gateway, 1.1.1.1, 8.8.8.8, 9.9.9.9)
sudo ptop

# Add custom targets
sudo ptop -t example.com -t 208.67.222.222

# Custom ping interval (milliseconds)
sudo ptop -i 500

# Record session for replay
sudo ptop -l

# List recorded sessions
ptop --list-logs

# Replay a session
ptop --replay ~/.ptop/logs/2024-01-29T15-42-17.jsonl.gz

# Replay at 10x speed
ptop --replay ~/.ptop/logs/2024-01-29T15-42-17.jsonl.gz --speed 10
```

## Keyboard Controls

### List View

| Key | Action |
|-----|--------|
| `q` / `Esc` | Quit |
| `↑` / `k` | Select previous target |
| `↓` / `j` | Select next target |
| `Enter` | Open detail view |
| `r` | Reset statistics |

### Detail View

| Key | Action |
|-----|--------|
| `Esc` / `Backspace` | Back to list |
| `↑` / `↓` | Navigate targets |
| `q` | Quit |
| `r` | Reset statistics |

### Replay Mode

| Key | Action |
|-----|--------|
| `Space` | Pause / Resume |
| `←` / `h` | Skip back 100 events |
| `→` / `l` | Skip forward 100 events |
| `+` / `=` | Speed up (2x) |
| `-` | Slow down (0.5x) |
| `q` | Quit |

## Default Targets

When run with `-d` (default: true), ptop monitors:

- **Gateway** - Auto-detected local network gateway
- **Cloudflare** - 1.1.1.1
- **Google** - 8.8.8.8
- **Quad9** - 9.9.9.9

## Data Storage

Session data is stored in `~/.ptop/`:

```
~/.ptop/
├── sessions/    # Session summaries (JSON, gzipped)
│   └── 2024-01-29T15-42-17.json.gz
└── logs/        # Raw ping logs for replay (JSONL, gzipped)
    └── 2024-01-29T15-42-17.jsonl.gz
```

## Quality Metrics

ptop calculates a MOS (Mean Opinion Score) based on latency, jitter, and packet loss:

| Grade | MOS | Quality |
|-------|-----|---------|
| A | ≥ 4.3 | Excellent |
| B | ≥ 4.0 | Good |
| C | ≥ 3.6 | Fair |
| D | ≥ 3.1 | Poor |
| F | < 3.1 | Bad |

## License

MIT
