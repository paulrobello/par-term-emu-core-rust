# Homebrew tap — publishing steps (run at a release cut)

This directory holds the Homebrew half of par-mux's installer paths. The
formula below is a TEMPLATE: the `url`/`sha256` lines are filled with the
release being published (the version appears three times, the SHA once per
rebuild). Publishing is an outward-facing action — do it at the release
cut, after the release artifacts exist:

1. Build the release archives (the release workflow's `par-mux-v<version>-<target>.tar.gz`
   set and the release-level `par-mux-v<version>-SHA256SUMS.txt`).
2. Compute the arm64 + intel macOS SHAs:
   `shasum -a 256 par-mux-v<V>-aarch64-apple-darwin.tar.gz` (and x86_64).
3. Put `Formula/par-mux.rb` (below) into the `paulrobello/homebrew-par-mux`
   tap repository with `<VERSION>` and the two `<SHA>` placeholders filled.
4. Push the tap. Users then install with:
   `brew tap paulrobello/par-mux && brew install par-mux`

The formula (two on_arch blocks, one per macOS target):

```ruby
class ParMux < Formula
  desc "tmux-control-mode-compatible terminal multiplexer daemon and client"
  homepage "https://github.com/paulrobello/par-term-emu-core-rust"
  version "<VERSION>"
  license "MIT"

  on_macos do
    if Hardware::CPU.intel?
      url "https://github.com/paulrobello/par-term-emu-core-rust/releases/download/v<VERSION>/par-mux-v<VERSION>-x86_64-apple-darwin.tar.gz"
      sha256 "<X86_SHA>"
    end
    if Hardware::CPU.arm?
      url "https://github.com/paulrobello/par-term-emu-core-rust/releases/download/v<VERSION>/par-mux-v<VERSION>-aarch64-apple-darwin.tar.gz"
      sha256 "<ARM_SHA>"
    end
  end

  # Homebrew chdirs into the archive's single top-level directory on
  # extract, so the binary installs by its own name despite the wrapper.
  def install
    bin.install "par-mux"
  end

  def caveats
    <<~EOS
      Completions: run `par-mux completion <shell>` and follow its output.
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/par-mux --version")
  end
end
```

The curl path lives in `scripts/install-par-mux.sh` and is served straight
from this repository's `main` branch (raw.githubusercontent URL in its
header) — no publishing step beyond the merge.
