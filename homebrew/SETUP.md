# Setting Up Homebrew Tap for ptop

## 1. Create the Homebrew Tap Repository

Create a new GitHub repository named `homebrew-tap` under your account/organization.

```bash
# Clone it locally
git clone https://github.com/freeeve/homebrew-tap
cd homebrew-tap

# Create Formula directory
mkdir Formula
```

## 2. Add the Formula

Copy `ptop.rb` to `Formula/ptop.rb` in your tap repository.

Before copying, update:
- Replace `freeeve` with your GitHub username/organization
- Update SHA256 hashes after first release (see step 4)

## 3. Set Up GitHub Token for Auto-Updates

For automatic formula updates on release:

1. Create a Personal Access Token with `repo` scope
2. Add it as a secret named `HOMEBREW_TAP_TOKEN` in your ptop repository

## 4. Update SHA256 Hashes

After creating a release, calculate SHA256 for each binary:

```bash
# Download and hash each release asset
curl -L https://github.com/freeeve/ptop/releases/download/v0.1.0/ptop-macos-aarch64.tar.gz | shasum -a 256
curl -L https://github.com/freeeve/ptop/releases/download/v0.1.0/ptop-macos-x86_64.tar.gz | shasum -a 256
curl -L https://github.com/freeeve/ptop/releases/download/v0.1.0/ptop-linux-x86_64.tar.gz | shasum -a 256
curl -L https://github.com/freeeve/ptop/releases/download/v0.1.0/ptop-linux-aarch64.tar.gz | shasum -a 256
```

Update the formula with the correct hashes.

## 5. Users Can Now Install

```bash
brew tap freeeve/tap
brew install ptop
```

## Alternative: Build from Source Formula

If you prefer Homebrew to build from source (no pre-built binaries needed):

```ruby
class Ptop < Formula
  desc "Network latency monitor with terminal UI - htop for ping"
  homepage "https://github.com/freeeve/ptop"
  url "https://github.com/freeeve/ptop/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "PLACEHOLDER_SOURCE_SHA256"
  license "MIT"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  def caveats
    <<~EOS
      ptop requires elevated privileges to send ICMP packets.
      Run with sudo: sudo ptop
    EOS
  end

  test do
    assert_match "ptop #{version}", shell_output("#{bin}/ptop --version")
  end
end
```

This approach is simpler but requires users to have Rust installed (Homebrew handles this automatically).
