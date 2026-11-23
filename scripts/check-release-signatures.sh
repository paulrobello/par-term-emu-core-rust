#!/usr/bin/env bash
#
# Verify that every given release artifact carries a minisign signature that
# verifies against the public key pinned in self_update.rs. Fails loudly
# listing anything unsigned or unverifiable — the mechanical guard against a
# cut shipping without authenticity sidecars (SEC-001).
#
# Usage: scripts/check-release-signatures.sh FILE|DIR [FILE|DIR...]
#   Directories are walked one level for consumable release assets; the
#   usual cut invocation is:
#     scripts/check-release-signatures.sh release-binaries mux-release web-frontend

set -euo pipefail

command -v minisign >/dev/null 2>&1 || { echo "check-release-signatures.sh: minisign is not installed" >&2; exit 2; }

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
pub=$(mktemp)
trap 'rm -f "$pub"' EXIT
printf 'untrusted comment: minisign public key 7D5A457F1AB85B55\nRWRVW7gaf0VafV4bihSeMbq2JiaTYTanOpdm5EkgpPk65edfjHsAnA+t\n' >"$pub"
grep -q 'RWRVW7gaf0VafV4bihSeMbq2JiaTYTanOpdm5EkgpPk65edfjHsAnA+t' "$repo_root/crates/par-term-emu-core/src/self_update.rs" \
  || { echo "check-release-signatures.sh: pinned key in self_update.rs differs from this script" >&2; exit 1; }

fail=0
checked=0
check_one() {
  local f=$1
  case "$f" in
    *.sha256|*.minisig|*SHA256SUMS*) return ;;  # sidecars, not signed payloads
  esac
  [ -f "$f" ] || return 0
  checked=$((checked + 1))
  if [ ! -f "${f}.minisig" ]; then
    echo "UNSIGNED: $f (no ${f}.minisig)" >&2
    fail=1
    return
  fi
  if ! minisign -V -q -p "$pub" -m "$f" -x "${f}.minisig" 2>/dev/null; then
    echo "BAD SIGNATURE: ${f}.minisig does not verify against the pinned key" >&2
    fail=1
  fi
}

[ "$#" -gt 0 ] || { echo "usage: $0 FILE|DIR [FILE|DIR...]" >&2; exit 2; }
for arg in "$@"; do
  if [ -d "$arg" ]; then
    for f in "$arg"/*; do check_one "$f"; done
  else
    check_one "$arg"
  fi
done

if [ "$checked" -eq 0 ]; then
  echo "check-release-signatures.sh: no artifacts found in the given paths" >&2
  exit 2
fi
if [ "$fail" -ne 0 ]; then
  echo "check-release-signatures.sh: FAILED (unsigned or unverifiable artifacts above)" >&2
  exit 1
fi
echo "release signatures ok: $checked artifact(s) verified against key 7D5A457F1AB85B55"
