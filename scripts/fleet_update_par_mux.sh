#!/usr/bin/env bash
#
# Update par-mux on the fleet hosts (minimac + lenny1/2/6) from a published
# GitHub release, restart each daemon, and verify. Encodes the per-host
# deployment models discovered 2026-11-24:
#   - minimac (macOS): the launchd job execs the Par-mux.app bundle binary;
#     deploy the CI Developer-ID-signed binary into the bundle WITHOUT
#     re-signing (the bundle seal is left stale by design — the inner
#     binary's team signature is what doctor's stable-identity reads) and
#     also refresh ~/.local/bin/par-mux. Restart via launchctl bootout/
#     bootstrap when the job is spawn-throttled.
#   - lenny1/2/6 (Linux, root): /usr/local/bin/par-mux, self-daemonized
#     (no service manager). Deploy the aarch64-linux-gnu binary and restart
#     with `par-mux --restart --no-attach` (a wedged daemon needs pkill first).
#
# Usage:
#   scripts/fleet_update_par_mux.sh [TAG]        # TAG defaults to the latest release
#   make fleet-par-mux-update TAG=v0.90.0
#
# Requirements on the CONTROLLING machine: gh (authed), minisign, ssh aliases
# for minimac/lenny1/lenny2/lenny6. Hosts need no tools beyond a shell.

set -euo pipefail

TAG="${1:-}"
REPO="paulrobello/par-term-emu-core-rust"
HOSTS="${FLEET_HOSTS:-minimac lenny1 lenny2 lenny6}"
PUBKEY='untrusted comment: minisign public key 7D5A457F1AB85B55
RWRVW7gaf0VafV4bihSeMbq2JiaTYTanOpdm5EkgpPk65edfjHsAnA+t'
MAC_HOSTS="minimac"
MAC_ASSET_ARCH="aarch64-apple-darwin"
LINUX_ASSET_ARCH="aarch64-unknown-linux-gnu"

if ! command -v gh >/dev/null 2>&1 || ! command -v minisign >/dev/null 2>&1; then
    echo "fleet_update: gh and minisign are required on the controlling machine" >&2
    exit 2
fi

if [[ -z "$TAG" ]]; then
    TAG="$(gh release list --repo "$REPO" --limit 20 --json tagName,isLatest \
        --jq '.[] | select(.isLatest) | .tagName')"
    [[ -n "$TAG" ]] || { echo "fleet_update: could not determine the latest release tag" >&2; exit 2; }
fi
echo "fleet_update: updating ${HOSTS} from ${TAG}"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Download + verify each platform's asset once, on the controlling machine.
verify_platform() { # $1 = asset arch
    local arch="$1" pub
    (cd "$WORK" && gh release download "$TAG" --repo "$REPO" \
        -p "par-mux-${TAG}-${arch}.tar.gz" \
        -p "par-mux-${TAG}-${arch}.tar.gz.minisig")
    pub="$WORK/release.pub"
    printf '%s\n' "$PUBKEY" > "$pub"
    minisign -V -q -p "$pub" -m "$WORK/par-mux-${TAG}-${arch}.tar.gz" \
        -x "$WORK/par-mux-${TAG}-${arch}.tar.gz.minisig" \
        || { echo "fleet_update: minisign verification FAILED for ${arch}" >&2; exit 1; }
    (cd "$WORK" && tar -xzf "par-mux-${TAG}-${arch}.tar.gz")
    echo "fleet_update: verified ${arch} asset for ${TAG}"
}

update_mac_host() { # $1 = host
    local host="$1" bin="$WORK/par-mux-${TAG}-${MAC_ASSET_ARCH}/par-mux"
    [[ -x "$bin" ]] || return 0
    echo "--- ${host}: deploy signed binary + restart ---"
    scp -q "$bin" "${host}:/tmp/par-mux-deploy" \
        && ssh -o BatchMode=yes "$host" '
            rm -f /Applications/Par-mux.app/Contents/MacOS/Par-mux
            cp /tmp/par-mux-deploy /Applications/Par-mux.app/Contents/MacOS/Par-mux
            chmod +x /Applications/Par-mux.app/Contents/MacOS/Par-mux
            rm -f ~/.local/bin/par-mux
            cp /tmp/par-mux-deploy ~/.local/bin/par-mux
            chmod +x ~/.local/bin/par-mux
            rm -f /tmp/par-mux-deploy
            launchctl bootout gui/$(id -u)/com.paulrobello.par-mux 2>/dev/null || true
            sleep 3
            PLIST=$(ls ~/Library/LaunchAgents/com.paulrobello.par-mux.plist /Library/LaunchAgents/com.paulrobello.par-mux.plist 2>/dev/null | head -1)
            launchctl bootstrap gui/$(id -u) "$PLIST"
            sleep 8
            par-mux --cmd version | head -1
            par-mux --cmd doctor | grep -i verdict | head -1'
    echo "--- ${host}: done"
}

update_linux_host() { # $1 = host
    local host="$1" bin="$WORK/par-mux-${TAG}-${LINUX_ASSET_ARCH}/par-mux"
    [[ -x "$bin" ]] || return 0
    echo "--- ${host}: deploy binary + restart ---"
    scp -q "$bin" "${host}:/tmp/par-mux-deploy" \
        && ssh -o BatchMode=yes "$host" '
            pkill -f "/usr/local/bin/par-mux" 2>/dev/null || true
            sleep 2
            cp /tmp/par-mux-deploy /usr/local/bin/par-mux
            chmod +x /usr/local/bin/par-mux
            rm -f /tmp/par-mux-deploy
            par-mux --restart --no-attach 2>&1 | tail -1
            sleep 4
            par-mux --cmd version | head -1'
    echo "--- ${host}: done"
}

verify_platform "$MAC_ASSET_ARCH"
verify_platform "$LINUX_ASSET_ARCH"

for host in $HOSTS; do
    case "$host" in
        minimac) update_mac_host "$host" ;;
        *)       update_linux_host "$host" ;;
    esac
done

echo "fleet_update: complete — verify each host with: ssh HOST par-mux --cmd version"
