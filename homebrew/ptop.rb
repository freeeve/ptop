class Ptop < Formula
  desc "Network latency monitor with terminal UI - htop for ping"
  homepage "https://github.com/freeeve/ptop"
  version "0.1.0"
  license "MIT"

  on_macos do
    on_arm do
      url "https://github.com/freeeve/ptop/releases/download/v#{version}/ptop-macos-aarch64.tar.gz"
      sha256 "PLACEHOLDER_SHA256_ARM64"
    end
    on_intel do
      url "https://github.com/freeeve/ptop/releases/download/v#{version}/ptop-macos-x86_64.tar.gz"
      sha256 "PLACEHOLDER_SHA256_X86_64"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/freeeve/ptop/releases/download/v#{version}/ptop-linux-x86_64.tar.gz"
      sha256 "PLACEHOLDER_SHA256_LINUX_X86_64"
    end
  end

  def install
    bin.install "ptop"
  end

  def caveats
    <<~EOS
      ptop requires elevated privileges to send ICMP packets.
      Run with sudo:
        sudo ptop

      Or on Linux, set capabilities:
        sudo setcap cap_net_raw=ep #{bin}/ptop
    EOS
  end

  test do
    assert_match "ptop #{version}", shell_output("#{bin}/ptop --version")
  end
end
