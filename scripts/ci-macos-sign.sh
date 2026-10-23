#!/usr/bin/env bash
#
# Sign a compiled macOS binary on a macOS runner: Developer ID + notarization
# when the APPLE_* credentials are configured, ad hoc otherwise.
#
# Port of oh-my-pi's scripts/ci-macos-sign.sh to native runner tooling
# (security + codesign + notarytool instead of rcodesign, which that script
# pins for its Linux cross-compile runners). Same credential contract, so the
# secret values copy over unchanged.
#
# The application firewall keys its per-binary inbound rule on the signature,
# so a Developer ID signature survives self-update swaps of the file, while an
# ad-hoc one does not.
#
# A bare Mach-O executable cannot be stapled (stapling only supports
# .app/.pkg/.dmg), so the notarization ticket is served online: Gatekeeper
# fetches it by cdhash on first assessment. curl downloads do not set the
# quarantine bit, so they never invoke Gatekeeper in the first place.
#
# Credentials (GitHub Actions secrets; all five or none):
#   APPLE_CERTIFICATE_P12        base64 of the Developer ID Application .p12 bundle
#   APPLE_CERTIFICATE_PASSWORD   password protecting that .p12
#   APPLE_API_KEY_ID             App Store Connect API key id (the "Key ID")
#   APPLE_API_ISSUER_ID          App Store Connect API issuer id (UUID)
#   APPLE_API_KEY                base64 of the App Store Connect .p8 private key
#
# Usage: scripts/ci-macos-sign.sh <path-to-binary> [more binaries...]

set -euo pipefail

if [[ "$(uname -s)" != Darwin ]]; then
	echo "ci-macos-sign: runs on macOS runners" >&2
	exit 1
fi

if [[ $# -eq 0 ]]; then
	echo "usage: ci-macos-sign.sh <path-to-binary> [more binaries...]" >&2
	exit 1
fi
for binary in "$@"; do
	if [[ ! -f "$binary" ]]; then
		echo "ci-macos-sign: binary not found: $binary" >&2
		exit 1
	fi
done

set_vars=()
missing=()
for var in APPLE_CERTIFICATE_P12 APPLE_CERTIFICATE_PASSWORD APPLE_API_KEY_ID APPLE_API_ISSUER_ID APPLE_API_KEY; do
	if [[ -n "${!var:-}" ]]; then set_vars+=("$var"); else missing+=("$var"); fi
done
if ((${#set_vars[@]} && ${#missing[@]})); then
	echo "ci-macos-sign: incomplete credentials, missing: ${missing[*]}" >&2
	exit 1
fi

WORKDIR="$(mktemp -d)"
KEYCHAIN="$WORKDIR/sign.keychain-db"
trap 'rm -rf "$WORKDIR"' EXIT

if ((${#set_vars[@]} == 0)); then
	echo "ci-macos-sign: no APPLE_* credentials; signing ad hoc"
	for binary in "$@"; do
		codesign --force -s - "$binary"
	done
	exit 0
fi

CERT_PATH="$WORKDIR/cert.p12"
API_KEY_PATH="$WORKDIR/api-key.p8"

echo "ci-macos-sign: decoding credentials"
(
	umask 077
	printf '%s' "$APPLE_CERTIFICATE_P12" | base64 --decode >"$CERT_PATH"
	printf '%s' "$APPLE_API_KEY" | base64 --decode >"$API_KEY_PATH"
)

echo "ci-macos-sign: importing the Developer ID identity into a temp keychain"
KEYCHAIN_PASS="$(uuidgen)"
security create-keychain -p "$KEYCHAIN_PASS" "$KEYCHAIN"
security set-keychain-settings -lut 21600 "$KEYCHAIN"
security import "$CERT_PATH" -k "$KEYCHAIN" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
security set-key-partition-list -S apple-tool:,apple: -k "$KEYCHAIN_PASS" "$KEYCHAIN" >/dev/null
security list-keychain -d user -s "$KEYCHAIN" login.keychain-db
IDENTITY="$(security find-identity -v -p codesigning "$KEYCHAIN" \
	| awk '/Developer ID Application/ {print $2; exit}')"
if [[ -z "$IDENTITY" ]]; then
	echo "ci-macos-sign: no Developer ID Application identity in the imported p12" >&2
	exit 1
fi

for binary in "$@"; do
	echo "ci-macos-sign: signing $(basename "$binary")"
	codesign --force --options runtime --timestamp -s "$IDENTITY" "$binary"
	codesign --verify --strict "$binary"
done

echo "ci-macos-sign: submitting for notarization"
PROFILE="par-term-notary-$$"
xcrun notarytool store-credentials "$PROFILE" \
	--key "$API_KEY_PATH" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER_ID"
for binary in "$@"; do
	zip_path="$WORKDIR/$(basename "$binary").zip"
	(cd "$(dirname "$binary")" && zip -q -X "$zip_path" "$(basename "$binary")")
	for attempt in 1 2 3; do
		if xcrun notarytool submit "$zip_path" --keychain-profile "$PROFILE" --wait \
			--timeout 30m >/dev/null 2>&1; then
			echo "ci-macos-sign: notarized $(basename "$binary")"
			echo "ci-macos-sign: note - a bare Mach-O cannot be stapled; the ticket is verified online."
			break
		fi
		echo "ci-macos-sign: notarization attempt $attempt for $(basename "$binary") failed" >&2
		if [[ "$attempt" == 3 ]]; then
			xcrun notarytool submit "$zip_path" --keychain-profile "$PROFILE" --wait \
				--timeout 30m >&2 || true
			exit 1
		fi
		sleep 30
	done
done
