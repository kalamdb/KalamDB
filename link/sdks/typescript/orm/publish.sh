#!/usr/bin/env bash
# publish.sh - Publish the @kalamdb/orm TypeScript SDK to npm
#
# Usage:
#   ./publish.sh [OPTIONS]
#
# Options:
#   --force            Force republish: unpublish existing version first (only works within 72h)
#   --version VERSION  Override version (default: read from package.json)
#   --skip-build       Skip the build step (use existing dist/)
#   --dry-run          Run everything except the actual npm publish
#   --otp CODE         One-time password for 2FA-protected accounts
#
# Environment:
#   NODE_AUTH_TOKEN    npm auth token (required for publishing; skipped in --dry-run)
#
# Example (local):
#   NPM_TOKEN=npm_xxx NODE_AUTH_TOKEN=$NPM_TOKEN ./publish.sh
#
# Example (force republish):
#   NODE_AUTH_TOKEN=$NPM_TOKEN ./publish.sh --force

set -euo pipefail

# ─── Resolve the script's own directory ──────────────────────────────────────
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SDK_DIR="$SCRIPT_DIR"

# ─── Load .env early (before everything else) ────────────────────────────────
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

# ─── Defaults ────────────────────────────────────────────────────────────────
FORCE_PUBLISH=false
SKIP_BUILD=false
DRY_RUN=false
VERSION_OVERRIDE=""
OTP_CODE=""

# ─── Parse arguments ─────────────────────────────────────────────────────────
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
    *)
      echo "❌ Unknown option: $1"
      echo "Usage: $0 [--force] [--version VERSION] [--skip-build] [--dry-run] [--otp CODE]"
      exit 1
      ;;
  esac
done

# ─── Read version and package name ───────────────────────────────────────────
PACKAGE_JSON="$SDK_DIR/package.json"
if [[ ! -f "$PACKAGE_JSON" ]]; then
  echo "❌ Could not find package.json at: $PACKAGE_JSON"
  exit 1
fi

PACKAGE_NAME="$(node -p "JSON.parse(require('fs').readFileSync(process.argv[1], 'utf8')).name" "$PACKAGE_JSON")"
if [[ -n "$VERSION_OVERRIDE" ]]; then
  VERSION="$VERSION_OVERRIDE"
  echo "📌 Using overridden version: $VERSION"
else
  VERSION="$(node -p "JSON.parse(require('fs').readFileSync(process.argv[1], 'utf8')).version" "$PACKAGE_JSON")"
  echo "📋 Version read from package.json: $VERSION"
fi

PACKAGE_REGISTRY_URL="${PUBLISH_REGISTRY_URL}/${PACKAGE_NAME}"
PACKAGE_PAGE_URL=""
if [[ "$PUBLISH_REGISTRY_URL" == "https://registry.npmjs.org" ]]; then
  PACKAGE_PAGE_URL="https://www.npmjs.com/package/${PACKAGE_NAME}"
fi

# ─── Resolve required peer dependency (@kalamdb/client) ──────────────────────
CLIENT_PACKAGE_NAME="$(node -p "Object.keys(JSON.parse(require('fs').readFileSync(process.argv[1], 'utf8')).peerDependencies ?? {})[0] || ''" "$PACKAGE_JSON")"
if [[ -z "$CLIENT_PACKAGE_NAME" ]]; then
  echo "❌ Could not determine the required peer package from peerDependencies."
  exit 1
fi
CLIENT_REGISTRY_URL="${PUBLISH_REGISTRY_URL}/${CLIENT_PACKAGE_NAME}"

echo ""
echo "══════════════════════════════════════════════════════"
echo "  $PACKAGE_NAME $PUBLISH_REGISTRY_NAME publish"
echo "  Version   : $VERSION"
echo "  Force     : $FORCE_PUBLISH"
echo "  Dry-run   : $DRY_RUN"
echo "  Skip-build: $SKIP_BUILD"
echo "══════════════════════════════════════════════════════"
echo ""

cd "$SDK_DIR"

# ─── Build ────────────────────────────────────────────────────────────────────
if [[ "$SKIP_BUILD" == "false" ]]; then
  echo "📦 Installing npm dependencies..."
  npm ci --legacy-peer-deps 2>/dev/null || npm install --legacy-peer-deps

  echo "🔨 Building SDK..."
  npm run build

  echo "✅ Build complete"
  ls -la dist/
else
  echo "⏭️  Skipping build (--skip-build)"
  if [[ ! -d "$SDK_DIR/dist" ]]; then
    echo "❌ dist/ directory not found. Run without --skip-build first."
    exit 1
  fi
fi

# ─── Determine npm dist-tags for pre-release versions ────────────────────────
NPM_TAG_FLAG="--tag latest"
PRERELEASE_TAG=""
if [[ "$VERSION" == *"-"* ]]; then
  PRERELEASE_LABEL="$(echo "$VERSION" | sed 's/^[^-]*-//' | sed 's/[.0-9]*$//' | tr -d '[:digit:]')"
  PRERELEASE_LABEL="${PRERELEASE_LABEL:-next}"
  PRERELEASE_TAG="$PRERELEASE_LABEL"
  echo "🏷️  Pre-release version detected — publishing to latest and adding dist-tag: $PRERELEASE_TAG"
fi

# ─── Check peer dependency is already published ───────────────────────────────
# ORM has independent versioning from @kalamdb/client; just verify the package exists on npm.
if ! npm view "$CLIENT_PACKAGE_NAME" version --silent $REGISTRY_FLAG >/dev/null 2>&1; then
  echo "❌ ${CLIENT_PACKAGE_NAME} is not published on $PUBLISH_REGISTRY_NAME. Publish the client package first."
  echo "   registry API: $CLIENT_REGISTRY_URL"
  exit 1
fi
echo "✅ Peer dependency ${CLIENT_PACKAGE_NAME} is available on $PUBLISH_REGISTRY_NAME."

# ─── Dry-run early exit ───────────────────────────────────────────────────────
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

# ─── Validate auth token ─────────────────────────────────────────────────────
if [[ -z "${NODE_AUTH_TOKEN:-}" ]]; then
  echo ""
  echo "❌ NODE_AUTH_TOKEN is not set."
  echo "   Either export it or add it to $SDK_DIR/.env:"
  echo "     NODE_AUTH_TOKEN=npm_xxxxxxxx"
  exit 1
fi

# ─── Write a local .npmrc with the auth token ────────────────────────────────
LOCAL_NPMRC="$SDK_DIR/.npmrc"
cleanup_npmrc() { rm -f "$LOCAL_NPMRC"; }
trap cleanup_npmrc EXIT
npm config set "//${PUBLISH_REGISTRY_HOST}/:_authToken" "${NODE_AUTH_TOKEN}" --location=project

# ─── Publish ──────────────────────────────────────────────────────────────────
echo ""
echo "🚀 Publishing $PACKAGE_NAME@$VERSION to $PUBLISH_REGISTRY_NAME..."
PUBLISH_SCRIPTS_FLAG=""
if [[ "$SKIP_BUILD" == "true" ]]; then
  PUBLISH_SCRIPTS_FLAG="--ignore-scripts"
fi

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

add_latest_dist_tag() {
  # shellcheck disable=SC2086
  npm dist-tag add "$PACKAGE_NAME@$VERSION" latest $OTP_FLAG $REGISTRY_FLAG
  echo "✅ Set dist-tag 'latest' for $PACKAGE_NAME@$VERSION"
}

if npm view "$PACKAGE_NAME@$VERSION" version --silent $REGISTRY_FLAG >/dev/null 2>&1; then
  if [[ "$FORCE_PUBLISH" == "true" ]]; then
    echo "⚠️  Version $VERSION exists. Force publish enabled — attempting to unpublish..."
    if npm unpublish "$PACKAGE_NAME@$VERSION" --force $REGISTRY_FLAG 2>/dev/null; then
      echo "✅ Successfully unpublished $PACKAGE_NAME@$VERSION"
      # shellcheck disable=SC2086
      npm publish $ACCESS_FLAG $NPM_TAG_FLAG $PUBLISH_SCRIPTS_FLAG $OTP_FLAG $REGISTRY_FLAG
      add_prerelease_dist_tag
      echo "✅ Successfully republished $PACKAGE_NAME@$VERSION to $PUBLISH_REGISTRY_NAME!"
    else
      echo "❌ Failed to unpublish (version may be >72 hours old)"
      echo "💡 Tip: npm doesn't allow unpublishing after 72 hours."
      echo "    Use a different version number in package.json and try again."
      exit 1
    fi
  else
    echo "ℹ️  Version $VERSION already exists on $PUBLISH_REGISTRY_NAME — updating dist-tags instead of republishing."
    add_latest_dist_tag
    add_prerelease_dist_tag
    exit 0
  fi
else
  # shellcheck disable=SC2086
  if npm publish $ACCESS_FLAG $NPM_TAG_FLAG $PUBLISH_SCRIPTS_FLAG $OTP_FLAG $REGISTRY_FLAG 2>&1; then
    add_prerelease_dist_tag
    echo "✅ Successfully published $PACKAGE_NAME@$VERSION to $PUBLISH_REGISTRY_NAME!"
    if [[ -n "$PACKAGE_PAGE_URL" ]]; then
      echo "   package page: $PACKAGE_PAGE_URL"
    fi
    echo "   registry API: $PACKAGE_REGISTRY_URL"
  else
    NPM_PUBLISH_EXIT=$?
    echo ""
    echo "❌ npm publish failed (exit $NPM_PUBLISH_EXIT)"
    echo "   Common causes:"
    echo "   • 2FA required: pass --otp <code> for your authenticator app."
    echo "     → For CI/CD, use an npm 'automation' token (bypasses 2FA)."
    echo "   • Version was previously published and unpublished — npm permanently blocks republishing the same version."
    echo "     → Bump the version in package.json and try again."
    echo "   • Auth token expired or lacks publish permissions."
    echo "     → Regenerate your npm token and update .env"
    exit $NPM_PUBLISH_EXIT
  fi
fi
