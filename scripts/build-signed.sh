#!/bin/sh
# Signed production build without exporting APPLE_SIGNING_IDENTITY by hand.
#
# Picks the "Developer ID Application" certificate from the login keychain.
# An identity already in the environment wins, which is how CI passes its own.
# Refuses to fall back to an ad-hoc build: that one would lose its keychain
# grants on the next rebuild (see "Signing local builds" in CLAUDE.md).
set -eu

if [ -z "${APPLE_SIGNING_IDENTITY:-}" ]; then
	identities=$(security find-identity -v -p codesigning |
		sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | sort -u)
	count=$(printf '%s' "$identities" | grep -c . || true)

	case "$count" in
	0)
		echo "No \"Developer ID Application\" certificate found in the keychain." >&2
		echo "Install one, or set APPLE_SIGNING_IDENTITY yourself." >&2
		exit 1
		;;
	1)
		APPLE_SIGNING_IDENTITY=$identities
		;;
	*)
		echo "More than one \"Developer ID Application\" certificate found:" >&2
		echo "$identities" | sed 's/^/  /' >&2
		echo "Pick one with APPLE_SIGNING_IDENTITY=\"…\" pnpm build:mac" >&2
		exit 1
		;;
	esac
	export APPLE_SIGNING_IDENTITY
fi

echo "Signing with: $APPLE_SIGNING_IDENTITY"
exec pnpm tauri build "$@"
