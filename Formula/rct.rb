# frozen_string_literal: true

# Homebrew formula for RCT (Rust Claude Terminal)
# Install: brew install postrv/tap/rct
# Or from local: brew install --build-from-source ./Formula/rct.rb
class Rct < Formula
  desc "High-performance Rust CLI for Claude API"
  homepage "https://github.com/postrv/rct"
  license any_of: ["MIT", "Apache-2.0"]
  head "https://github.com/postrv/rct.git", branch: "main"

  # Stable release URL will be filled in by release workflow
  # url "https://github.com/postrv/rct/archive/refs/tags/v#{version}.tar.gz"
  # sha256 "PLACEHOLDER"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  def caveats
    <<~EOS
      RCT requires an Anthropic API key. Set it via:
        export ANTHROPIC_API_KEY="your-api-key"

      Or create a config file at ~/.config/rct/config.toml:
        [api]
        key = "your-api-key"

      For more information, see:
        https://github.com/postrv/rct#configuration
    EOS
  end

  test do
    # Test that the binary runs and shows version
    assert_match version.to_s, shell_output("#{bin}/rct --version")

    # Test that it errors appropriately without API key
    output = shell_output("#{bin}/rct --help")
    assert_match "Rust Claude Terminal", output
  end
end
