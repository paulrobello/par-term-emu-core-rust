#!/usr/bin/env bash
# Installer for the par-term-emu-core-rust standalone release binaries.
#
# Typical use (macOS / Linux):
#   curl -fsSL https://raw.githubusercontent.com/paulrobello/par-term-emu-core-rust/main/install.sh | bash
#
# Installs par-term-streamer and/or par-mux into ~/.cargo/bin (or --bin-dir),
# verifying every download against its release SHA-256 before anything is
# moved into place. Never touches launchd or any other service.
#
# Self-test (no network): bash install.sh --self-test

set -euo pipefail

REPO="paulrobello/par-term-emu-core-rust"
API_LATEST="https://api.github.com/repos/${REPO}/releases/latest"
DL_BASE="https://github.com/${REPO}/releases/download"

log() { printf 'install.sh: %s\n' "$*" >&2; }
die() { log "error: $*"; exit 1; }

usage() {
  cat >&2 <<'EOF'
Usage: install.sh [--streamer | --mux] [--version vX.Y.Z] [--bin-dir DIR]

Options:
  --streamer        Install only par-term-streamer (default: both binaries)
  --mux             Install only par-mux (default: both binaries)
  --version vX.Y.Z  Pin a release (default: latest GitHub release)
  --bin-dir DIR     Install destination (default: ~/.cargo/bin)
  --self-test       Run offline self-tests of the pure helper functions
  -h, --help        Show this help
EOF
}

# ---------------------------------------------------------------- pure helpers

# Strip a leading v and require strict X.Y.Z. Prints the normalized version.
normalize_version() {
  local v="${1#v}"
  if ! [[ $v =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    return 1
  fi
  printf '%s' "$v"
}

# Map os/arch (uname style) to the par-term-streamer release asset name.
map_streamer_asset() {
  case "$1/$2" in
    darwin/arm64)  printf 'par-term-streamer-macos-aarch64' ;;
    darwin/x86_64) printf 'par-term-streamer-macos-x86_64' ;;
    linux/arm64)   printf 'par-term-streamer-linux-aarch64' ;;
    linux/x86_64)  printf 'par-term-streamer-linux-x86_64' ;;
    *) return 1 ;;
  esac
}

# Map os/arch to the par-mux release archive name (packaged top directory).
map_mux_archive() {
  local target
  case "$1/$2" in
    darwin/arm64)  target=aarch64-apple-darwin ;;
    darwin/x86_64) target=x86_64-apple-darwin ;;
    linux/arm64)   target=aarch64-unknown-linux-gnu ;;
    linux/x86_64)  target=x86_64-unknown-linux-gnu ;;
    *) return 1 ;;
  esac
  printf 'par-mux-v%s-%s.tar.gz' "$3" "$target"
}

# Archive member path for the par-mux binary: <top>/par-mux where the
# top directory is the archive name without its .tar.gz suffix.
mux_member_path() {
  printf '%s/par-mux' "${1%.tar.gz}"
}

# Extract the shasum entry for exactly $1 from sha256sum-format text on stdin.
# Fails on a missing, malformed, or duplicate entry.
parse_sums() {
  local name=$1 line digest asset found=""
  local n=0
  set -f
  while IFS= read -r line || [ -n "$line" ]; do
    # shellcheck disable=SC2086  # word split is the point: fields are hex digests and fixed asset names
    set -- $line
    digest=${1-}
    asset=${2-}
    [ -n "$digest" ] || continue
    [ "${asset#\*}" = "$name" ] || continue
    n=$((n + 1))
    if [ -n "$found" ] || [[ ! $digest =~ ^[0-9a-fA-F]{64}$ ]]; then
      set +f
      return 1
    fi
    found=$digest
  done
  set +f
  if [ "$n" -eq 0 ]; then
    return 1
  fi
  printf '%s' "$found"
}

# Parse tag_name from GitHub release JSON on stdin. Prints the tag or fails.
parse_tag_name() {
  local tag
  tag=$(sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)
  [ -n "$tag" ] || return 1
  printf '%s' "$tag"
}

# ------------------------------------------------------------- self-test mode

self_test() {
  local failures=0
  check() { # $1=description $2=expected $3=actual
    if [ "$2" = "$3" ]; then
      printf 'ok   %s\n' "$1"
    else
      printf 'FAIL %s: expected %q, got %q\n' "$1" "$2" "$3"
      failures=$((failures + 1))
    fi
  }
  check_err() { # $1=description $2=command...
    if "$@" >/dev/null 2>&1; then
      printf 'FAIL %s: expected failure, got success\n' "$1"
      failures=$((failures + 1))
    else
      printf 'ok   %s\n' "$1"
    fi
  }
  check_fails() { # $1=description $2=function $3=name passed to it $4=stdin printf format, $5+ printf args
    local desc=$1 fn=$2 name=$3
    # shellcheck disable=SC2059  # the format is an inline self-test fixture, not user data
    if printf "$4" "${@:5}" | "$fn" "$name" >/dev/null 2>&1; then
      printf 'FAIL %s: expected failure, got success\n' "$desc"
      failures=$((failures + 1))
    else
      printf 'ok   %s\n' "$desc"
    fi
  }

  # Platform -> streamer asset: every name deployment.yml uploads.
  check "darwin arm64 streamer" "par-term-streamer-macos-aarch64" "$(map_streamer_asset darwin arm64)"
  check "darwin x86_64 streamer" "par-term-streamer-macos-x86_64" "$(map_streamer_asset darwin x86_64)"
  check "linux arm64 streamer" "par-term-streamer-linux-aarch64" "$(map_streamer_asset linux arm64)"
  check "linux x86_64 streamer" "par-term-streamer-linux-x86_64" "$(map_streamer_asset linux x86_64)"
  check_err "unsupported platform streamer" map_streamer_asset windows x86_64
  check_err "unsupported arch streamer" map_streamer_asset linux i686

  # Platform -> mux archive: every name deployment.yml uploads.
  check "darwin arm64 mux" "par-mux-v1.2.3-aarch64-apple-darwin.tar.gz" "$(map_mux_archive darwin arm64 1.2.3)"
  check "darwin x86_64 mux" "par-mux-v1.2.3-x86_64-apple-darwin.tar.gz" "$(map_mux_archive darwin x86_64 1.2.3)"
  check "linux arm64 mux" "par-mux-v1.2.3-aarch64-unknown-linux-gnu.tar.gz" "$(map_mux_archive linux arm64 1.2.3)"
  check "linux x86_64 mux" "par-mux-v1.2.3-x86_64-unknown-linux-gnu.tar.gz" "$(map_mux_archive linux x86_64 1.2.3)"
  check_err "unsupported platform mux" map_mux_archive windows x86_64 1.2.3

  # Archive top directory / member path (mux_archive_uses_packaged_top_directory).
  check "mux member path" "par-mux-v9.9.9-aarch64-apple-darwin/par-mux" \
    "$(mux_member_path par-mux-v9.9.9-aarch64-apple-darwin.tar.gz)"

  # Version normalization.
  check "normalize v-prefixed" "1.2.3" "$(normalize_version v1.2.3)"
  check "normalize bare" "1.2.3" "$(normalize_version 1.2.3)"
  check_err "reject two-component version" normalize_version 1.2
  check_err "reject non-numeric version" normalize_version abc

  # Checksum-file parsing (par-mux-v<V>-SHA256SUMS.txt shape).
  local digest
  digest=ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad
  check "sums selects exact asset" "$digest" \
    "$(printf '%s  other-asset.tar.gz\n%s  wanted.tar.gz\n' "b${digest:1}" "$digest" | parse_sums wanted.tar.gz)"
  check "sums accepts binary-marker form" "$digest" \
    "$(printf '%s  wanted.tar.gz\n' "$digest" | parse_sums wanted.tar.gz)"
  check_fails "sums missing asset" parse_sums wanted.tar.gz '%s  other-asset.tar.gz\n' "$digest"
  check_fails "sums malformed digest" parse_sums wanted.tar.gz 'short  wanted.tar.gz\n'
  check_fails "sums duplicate entry" parse_sums wanted.tar.gz \
    '%s  wanted.tar.gz\n%s  wanted.tar.gz\n' "$digest" "$digest"

  # tag_name parsing from release JSON.
  check "parse tag_name" "v1.2.3" \
    "$(printf '{"html_url":"x","tag_name":"v1.2.3","assets":[]}' | parse_tag_name)"
  check_fails "parse missing tag_name" parse_tag_name ignored '{}\n'

  if [ "$failures" -eq 0 ]; then
    printf 'self-test: all checks passed\n'
    return 0
  fi
  printf 'self-test: %d check(s) failed\n' "$failures" >&2
  return 1
}

# -------------------------------------------------------------- network layer

fetch() { # $1=url $2=destination; returns nonzero on download failure
  case "$1" in
    https://*) ;;
    *) die "refusing non-HTTPS URL: $1" ;;
  esac
  curl --fail --silent --show-error --location \
    --proto '=https' --proto-redir '=https' \
    --connect-timeout 15 --max-time 600 \
    --user-agent par-term-installer \
    --output "$2" "$1"
}

sha256_of() { # $1=file -> digest on stdout
  if [ "$(uname -s)" = "Darwin" ]; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}

# --------------------------------------------------------------- install core

install_binary() { # $1=staged-file $2=destination
  local dest=$2 dir
  dir=$(dirname "$dest")
  mkdir -p "$dir"
  if [ -e "$dest" ]; then
    mv -f "$dest" "${dest}.previous"
    log "kept previous binary at ${dest}.previous"
  fi
  mv "$1" "$dest"
  chmod 755 "$dest"
  log "installed $dest"
}

install_one() { # $1=streamer|mux  $2=version $3=os $4=arch $5=dest-dir $6=tmpdir
  local kind=$1 version=$2 os=$3 arch=$4 dest_dir=$5 tmp=$6
  local asset sidecar dest
  if [ "$kind" = "streamer" ]; then
    asset=$(map_streamer_asset "$os" "$arch") || die "unsupported platform: $os/$arch (see ${DL_BASE})"
    sidecar="${asset}.sha256"
    dest="${dest_dir}/par-term-streamer"
  else
    asset=$(map_mux_archive "$os" "$arch" "$version") || die "unsupported platform: $os/$arch (see ${DL_BASE})"
    sidecar="par-mux-v${version}-SHA256SUMS.txt"
    dest="${dest_dir}/par-mux"
  fi

  log "fetching $asset (v$version, $os/$arch)"
  local sums_file asset_file expected actual member
  sums_file="${tmp}/${sidecar}"
  if ! fetch "${DL_BASE}/v${version}/${sidecar}" "$sums_file"; then
    if [ "$kind" = "streamer" ]; then
      die "no checksum sidecar ${sidecar} for this release; releases older than the sidecar feature ship no sidecar — install manually from https://github.com/${REPO}/releases"
    fi
    die "checksum file ${sidecar} not found for release v${version}"
  fi
  expected=$(parse_sums "$asset" <"$sums_file") || die "checksum file has no valid entry for ${asset}"

  asset_file="${tmp}/${asset}"
  fetch "${DL_BASE}/v${version}/${asset}" "$asset_file" || die "download failed: ${asset}"
  actual=$(sha256_of "$asset_file")
  if [ "$actual" != "$expected" ]; then
    die "SHA-256 mismatch for ${asset}: expected ${expected}, got ${actual}; nothing was installed"
  fi

  if [ "$kind" = "mux" ]; then
    member=$(mux_member_path "$asset")
    mkdir -p "${tmp}/extract"
    tar -xzf "$asset_file" -C "${tmp}/extract" "$member" || die "could not extract ${member} from ${asset}"
    install_binary "${tmp}/extract/${member}" "$dest"
  else
    install_binary "$asset_file" "$dest"
  fi
}

main() {
  local want_streamer=false want_mux=false pinned="" bin_dir="${HOME}/.cargo/bin"
  while [ $# -gt 0 ]; do
    case "$1" in
      --streamer) want_streamer=true ;;
      --mux) want_mux=true ;;
      --version)
        [ $# -ge 2 ] || die "--version requires a value"
        pinned=$2
        shift
        ;;
      --version=*) pinned=${1#--version=} ;;
      --bin-dir)
        [ $# -ge 2 ] || die "--bin-dir requires a value"
        bin_dir=$2
        shift
        ;;
      --bin-dir=*) bin_dir=${1#--bin-dir=} ;;
      --self-test)
        if self_test; then exit 0; else exit 1; fi ;;
      -h | --help) usage; exit 0 ;;
      *) usage; die "unknown argument: $1" ;;
    esac
    shift
  done
  if $want_streamer && $want_mux; then
    die "--streamer and --mux are mutually exclusive"
  fi
  if ! $want_streamer && ! $want_mux; then
    want_streamer=true
    want_mux=true
  fi

  local os arch version
  os=$(uname -s)
  case "$os" in
    Darwin) os=darwin ;;
    Linux) os=linux ;;
    *) die "unsupported OS: $(uname -s); on Windows use install.ps1 (irm .../install.ps1 | iex)" ;;
  esac
  arch=$(uname -m)
  case "$arch" in
    arm64 | aarch64) arch=arm64 ;;
    x86_64) arch=x86_64 ;;
    *) die "unsupported architecture: $(uname -m); supported: arm64/aarch64, x86_64" ;;
  esac

  if [ -n "$pinned" ]; then
    version=$(normalize_version "$pinned") || die "invalid --version value: $pinned (expected vX.Y.Z)"
  else
    log "resolving latest release"
    local json tag
    json=$(curl --fail --silent --show-error --proto '=https' --proto-redir '=https' \
      --connect-timeout 15 --max-time 60 \
      --user-agent par-term-installer "$API_LATEST") || die "could not fetch the latest release from ${API_LATEST}"
    tag=$(printf '%s' "$json" | parse_tag_name) || die "could not parse tag_name from the release API response"
    version=$(normalize_version "$tag") || die "release tag is not a plain version: ${tag}"
  fi

  local tmp
  tmp=$(mktemp -d) || die "could not create a temporary directory"
  trap 'rm -rf "$tmp"' EXIT

  if $want_streamer; then
    install_one streamer "$version" "$os" "$arch" "$bin_dir" "$tmp"
  fi
  if $want_mux; then
    install_one mux "$version" "$os" "$arch" "$bin_dir" "$tmp"
  fi

  case ":${PATH}:" in
    *":${bin_dir}:"*) ;;
    *) log "warning: ${bin_dir} is not on your PATH; add it to your shell profile" ;;
  esac

  log ""
  log "done (v${version}). Next steps:"
  if $want_streamer; then
    log "  par-term-streamer: ${bin_dir}/par-term-streamer"
    log "    serve the web terminal:  par-term-streamer --port 8099  (then open http://127.0.0.1:8099)"
  fi
  if $want_mux; then
    log "  par-mux: ${bin_dir}/par-mux  (running 'par-mux' starts the daemon)"
  fi
}

main "$@"
