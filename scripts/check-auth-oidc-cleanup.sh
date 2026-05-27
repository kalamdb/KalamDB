#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

failed=0

check_absent() {
	local description="$1"
	local pattern="$2"
	shift 2

	if rg -n --hidden --glob '!target/**' --glob '!**/node_modules/**' "$pattern" "$@"; then
		echo "Found legacy auth code: ${description}" >&2
		failed=1
	fi
}

check_absent \
	"custom OIDC validator/discovery" \
	'OidcValidator|OidcConfig::discover|struct OidcConfig|reqwest::get' \
	backend/crates/kalamdb-auth/src/oidc

check_absent \
	"provider-family auth branches" \
	'provider_family|provider\.family|OAuthProvider::(Google|AzureAd|GitHub|Firebase|Okta|Auth0)' \
	backend/crates/kalamdb-auth/src \
	backend/crates/kalamdb-api/src \
	backend/crates/kalamdb-commons/src \
	backend/crates/kalamdb-system/src

check_absent \
	"plural OAuth provider metadata endpoint" \
	'/auth/oauth/providers|oauth_providers_handler|OAuthProviderInfo' \
	backend/crates/kalamdb-api/src \
	ui/src

if [[ "$failed" -ne 0 ]]; then
	exit 1
fi

echo "Auth OIDC cleanup guard passed."