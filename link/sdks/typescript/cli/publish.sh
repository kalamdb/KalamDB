#!/usr/bin/env bash
# publish.sh - Publish the @kalamdb/cli package to npm

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SDK_DIR="$SCRIPT_DIR"

if [[ -z "${NODE_AUTH_TOKEN:-}" ]]; then
  ENV_FILE="$SDK_DIR/.env"
  if [[ -f "$ENV_FILE" ]]; then
    NODE_AUTH_TOKEN="$(grep -E '^NODE_AUTH_TOKEN=' "$ENV_FILE" | head -n1 | cut -d'=' -f2- | tr -d '[:space:]')"
    [[ -n "$NODE_AUTH_TOKEN" ]] && echo "🔑 Loaded NODE_AUTH_TOKEN from .env"
  fi
fi
export NODE_AUTH_TOKEN

PUBLISH_REGISTRY_URL="${PUBLISH_REGISTRY_URL:-https://registry.npmjs.org}"
PUBLISH_REGISTRY_URL="${PUBLISH_REGISTRY_URL%/}"
PUBLISH_REGISTRY_NAME="${PUBLISH_REGISTRY_NAME:-npm}"
PUBLISH_ACCESS="${PUBLISH_ACCESS-public}"
PUBLISH_REGISTRY_HOST="${PUBLISH_REGISTRY_URL#https://}"
PUBLISH_REGISTRY_HOST="${PUBLISH_REGISTRY_HOST#http://}"
PUBLISH_REGISTRY_HOST="${PUBLISH_REGISTRY_HOST%/}"
REGISTRY_FLAG="--registry $PUBLISH_REGISTRY_URL"
ACCESS_FLAG=""
if [[ -n "$PUBLISH_ACCESS" ]]; then
  ACCESS_FLAG="--access $PUBLISH_ACCESS"
fi
LOCAL_NPMRC=""

cleanup_publish_artifacts() {
  if [[ -n "$LOCAL_NPMRC" ]]; then
    rm -f "$LOCAL_NPMRC"
  fi
}

trap cleanup_publish_artifacts EXIT

FORCE_PUBLISH=false
SKIP_BUILD=false
DRY_RUN=false
VERSION_OVERRIDE=""
OTP_CODE=""
PROVENANCE_MODE="${NPM_PROVENANCE_MODE:-auto}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --force)
      FORCE_PUBLISH=true
      shift
      ;;
    --version)
      VERSION_OVERRIDE="$2"
      shift 2
      ;;
    --skip-build)
      SKIP_BUILD=true
      shift
      ;;
    --dry-run)
      DRY_RUN=true
      shift
      ;;
    --otp)
      OTP_CODE="$2"
      shift 2
      ;;
    --provenance)
      PROVENANCE_MODE="always"
      shift
      ;;
    --no-provenance)
      PROVENANCE_MODE="never"
      shift
      ;;
    *)
      echo "❌ Unknown option: $1"
      echo "Usage: $0 [--force] [--version VERSION] [--skip-build] [--dry-run] [--otp CODE] [--provenance|--no-provenance]"
      exit 1
      ;;
  esac
done

supports_provenance() {
  [[ "${GITHUB_ACTIONS:-}" == "true" ]] \
    && [[ -n "${ACTIONS_ID_TOKEN_REQUEST_TOKEN:-}" ]] \
    && [[ -n "${ACTIONS_ID_TOKEN_REQUEST_URL:-}" ]]
}

PROVENANCE_FLAG=""
PROVENANCE_STATUS="disabled"
case "$PROVENANCE_MODE" in
  auto)
    if supports_provenance; then
      PROVENANCE_FLAG="--provenance"
      PROVENANCE_STATUS="enabled (GitHub Actions OIDC)"
    else
      PROVENANCE_STATUS="disabled (unsupported outside GitHub Actions OIDC)"
    fi
    ;;
  always)
    if ! supports_provenance; then
      echo "❌ --provenance requested, but automatic provenance is only supported in GitHub Actions with OIDC enabled."
      exit 1
    fi
    PROVENANCE_FLAG="--provenance"
    PROVENANCE_STATUS="enabled (forced)"
    ;;
  never)
    PROVENANCE_STATUS="disabled (forced)"
    ;;
  *)
    echo "❌ Invalid NPM_PROVENANCE_MODE: $PROVENANCE_MODE"
    echo "   Expected one of: auto, always, never"
    exit 1
    ;;
esac

PACKAGE_JSON="$SDK_DIR/package.json"
if [[ ! -f "$PACKAGE_JSON" ]]; then
  echo "❌ Could not find package.json at: $PACKAGE_JSON"
  exit 1
fi

if [[ -n "$VERSION_OVERRIDE" ]]; then
  VERSION="$VERSION_OVERRIDE"
  echo "📌 Using overridden version: $VERSION"
else
  VERSION="$(node -p "JSON.parse(require('fs').readFileSync(process.argv[1], 'utf8')).version" "$PACKAGE_JSON")"
  echo "📋 Version read from package.json: $VERSION"
fi

cd "$SDK_DIR"

if [[ "$SKIP_BUILD" == "false" ]]; then
  echo "🧪 Running package install and release asset validation..."
  KALAM_CLI_SKIP_RELEASE_ASSET_CHECK=true KALAM_CLI_VERSION="$VERSION" bash ./test.sh
  node scripts/validate-release-assets.js --version "$VERSION"
else
  echo "⏭️  Skipping validation (--skip-build)"
fi

PACKAGE_NAME="$(node -p "JSON.parse(require('fs').readFileSync(process.argv[1], 'utf8')).name" "$PACKAGE_JSON")"

echo ""
echo "══════════════════════════════════════════════════════"
echo "  $PACKAGE_NAME $PUBLISH_REGISTRY_NAME publish"
echo "  Version   : $VERSION"
echo "  Force     : $FORCE_PUBLISH"
echo "  Dry-run   : $DRY_RUN"
echo "  Skip-build: $SKIP_BUILD"
echo "  Provenance: $PROVENANCE_STATUS"
echo "══════════════════════════════════════════════════════"
echo ""

NPM_TAG_FLAG="--tag latest"
PRERELEASE_TAG=""
if [[ "$VERSION" == *"-"* ]]; then
  PRERELEASE_LABEL="$(echo "$VERSION" | sed 's/^[^-]*-//' | sed 's/[.0-9]*$//' | tr -d '[:digit:]')"
  PRERELEASE_LABEL="${PRERELEASE_LABEL:-next}"
  PRERELEASE_TAG="$PRERELEASE_LABEL"
  echo "🏷️  Pre-release version detected — publishing to latest and adding dist-tag: $PRERELEASE_TAG"
fi

if [[ "$DRY_RUN" == "true" ]]; then
  echo ""
  echo "🔍 Dry-run mode: skipping actual publish."
  echo "   Would publish: $PACKAGE_NAME@$VERSION${NPM_TAG_FLAG:+ ($NPM_TAG_FLAG)}"
  if [[ -n "$PRERELEASE_TAG" ]]; then
    echo "   Would also add dist-tag: $PRERELEASE_TAG"
  fi
  # shellcheck disable=SC2086
  npm publish $ACCESS_FLAG $NPM_TAG_FLAG --dry-run --ignore-scripts $REGISTRY_FLAG
  exit 0
fi

if [[ -z "${NODE_AUTH_TOKEN:-}" ]]; then
  echo ""
  echo "❌ NODE_AUTH_TOKEN is not set."
  echo "   Either export it or add it to $SDK_DIR/.env:"
  echo "     NODE_AUTH_TOKEN=npm_xxxxxxxx"
  exit 1
fi

LOCAL_NPMRC="$SDK_DIR/.npmrc"
npm config set "//${PUBLISH_REGISTRY_HOST}/:_authToken" "${NODE_AUTH_TOKEN}" --location=project

OTP_FLAG=""
if [[ -n "$OTP_CODE" ]]; then
  OTP_FLAG="--otp $OTP_CODE"
fi

add_prerelease_dist_tag() {
  if [[ -n "$PRERELEASE_TAG" ]]; then
    # shellcheck disable=SC2086
    npm dist-tag add "$PACKAGE_NAME@$VERSION" "$PRERELEASE_TAG" $OTP_FLAG $REGISTRY_FLAG
    echo "✅ Added dist-tag '$PRERELEASE_TAG' for $PACKAGE_NAME@$VERSION"
  fi
}

if npm view "$PACKAGE_NAME@$VERSION" version --silent $REGISTRY_FLAG >/dev/null 2>&1; then
  if [[ "$FORCE_PUBLISH" == "true" ]]; then
    echo "⚠️  Version $VERSION exists. Force publish enabled — attempting to unpublish..."
    if npm unpublish "$PACKAGE_NAME@$VERSION" --force $REGISTRY_FLAG 2>/dev/null; then
      echo "✅ Successfully unpublished $PACKAGE_NAME@$VERSION"
      # shellcheck disable=SC2086
      rm -rf dist
      npm publish $ACCESS_FLAG $NPM_TAG_FLAG --ignore-scripts $PROVENANCE_FLAG $OTP_FLAG $REGISTRY_FLAG
      add_prerelease_dist_tag
      echo "✅ Successfully republished $PACKAGE_NAME@$VERSION to $PUBLISH_REGISTRY_NAME!"
    else
      echo "❌ Failed to unpublish (version may be >72 hours old)"
      exit 1
    fi
  else
    echo "ℹ️  $PACKAGE_NAME@$VERSION already exists on $PUBLISH_REGISTRY_NAME; skipping publish."
  fi
else
  echo "🚀 Publishing $PACKAGE_NAME@$VERSION to $PUBLISH_REGISTRY_NAME..."
  # shellcheck disable=SC2086
  rm -rf dist
  npm publish $ACCESS_FLAG $NPM_TAG_FLAG --ignore-scripts $PROVENANCE_FLAG $OTP_FLAG $REGISTRY_FLAG
  add_prerelease_dist_tag
  echo "✅ Successfully published $PACKAGE_NAME@$VERSION to $PUBLISH_REGISTRY_NAME!"
fi