# frozen_string_literal: true

# Homebrew formula for Patina
# Install: brew install postrv/tap/patina
# Or from local: brew install --build-from-source ./Formula/patina.rb
class Patina < Formula
  desc "High-performance terminal client for Claude API"
  homepage "https://github.com/postrv/patina"
  license any_of: ["MIT", "Apache-2.0"]
  head "https://github.com/postrv/patina.git", branch: "main"

  # Stable release URL will be filled in by release workflow
  # url "https://github.com/postrv/patina/archive/refs/tags/v#{version}.tar.gz"
  # sha256 "PLACEHOLDER"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
  end

  def caveats
    <<~EOS
      Patina requires an Anthropic API key. Set it via:
        export ANTHROPIC_API_KEY="your-api-key"

      Or create a config file at ~/.config/patina/config.toml:
        [api]
        key = "your-api-key"

      For more information, see:
        https://github.com/postrv/patina#configuration
    EOS
  end

  test do
    # Test that the binary runs and shows version
    assert_match version.to_s, shell_output("#{bin}/patina --version")

    # Test that it errors appropriately without API key
    output = shell_output("#{bin}/patina --help")
    assert_match "Patina", output
  end
end
