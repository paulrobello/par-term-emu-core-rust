#!/usr/bin/env bash
# Sign release artifacts with minisign, writing <artifact>.minisig next to each.
# CI signs at the cut (deployment.yml, gated on the `release` environment);
# this script is the owner's manual fallback and uses the same pinned key.
#
# Usage: MINISIGN_SECRET_KEY=/path/to/minisign.key scripts/sign-release.sh FILE...
#
# The signature must verify against the public key pinned in
# crates/par-term-emu-core/src/self_update.rs (key ID 7D5A457F1AB85B55);
# each signature is verified against that pinned key before the script exits.

set -euo pipefail

key="${MINISIGN_SECRET_KEY:-}"
[ -n "$key" ] || { echo "sign-release.sh: set MINISIGN_SECRET_KEY to your minisign secret key file" >&2; exit 2; }
[ -f "$key" ] || { echo "sign-release.sh: secret key file not found: $key" >&2; exit 2; }
[ "$#" -gt 0 ] || { echo "usage: MINISIGN_SECRET_KEY=KEYFILE $0 FILE..." >&2; exit 2; }
command -v minisign >/dev/null 2>&1 || { echo "sign-release.sh: minisign is not installed" >&2; exit 2; }

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
pub=$(mktemp)
trap 'rm -f "$pub"' EXIT
printf 'untrusted comment: minisign public key 7D5A457F1AB85B55\nRWRVW7gaf0VafV4bihSeMbq2JiaTYTanOpdm5EkgpPk65edfjHsAnA+t\n' >"$pub"
grep -q 'RWRVW7gaf0VafV4bihSeMbq2JiaTYTanOpdm5EkgpPk65edfjHsAnA+t' "$repo_root/crates/par-term-emu-core/src/self_update.rs" \
  || { echo "sign-release.sh: pinned key in self_update.rs differs from this script" >&2; exit 1; }

for f in "$@"; do
  [ -f "$f" ] || { echo "sign-release.sh: not a file: $f" >&2; exit 2; }
  minisign -S -s "$key" -m "$f" -x "${f}.minisig" -t "par-term-emu-core-rust release artifact $(basename "$f")"
  minisign -V -q -p "$pub" -m "$f" -x "${f}.minisig" \
    || { echo "sign-release.sh: ${f}.minisig does not verify against the pinned key (wrong secret key?)" >&2; rm -f "${f}.minisig"; exit 1; }
  echo "signed: ${f}.minisig"
done
