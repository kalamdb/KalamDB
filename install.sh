#!/bin/sh
# KalamDB CLI Installer
# Usage: curl -fsSL https://kalamdb.org/install.sh | sh
#
# Flags:
#   --version <version>  Install an exact version (for example 0.5.0 or v0.5.0)
#   --pre-release       Install the latest GitHub prerelease
#   --help              Show usage
#
# Environment variables:
#   KALAM_INSTALL_DIR  - Installation directory (default: $HOME/.kalam/bin)
#   KALAM_VERSION      - Specific version to install (default: latest)
#   KALAM_PRE_RELEASE  - Set to 1 to install the latest prerelease
#   KALAM_NO_MODIFY_PATH - Set to 1 to skip PATH modification

if [ -z "${BASH_VERSION:-}" ]; then
    if command -v bash >/dev/null 2>&1; then
        case "$0" in
            sh|-sh|*/sh|dash|-dash|*/dash)
                ;;
            *)
                if [ -r "$0" ]; then
                    KALAM_INSTALLER_REEXEC=1 exec bash "$0" "$@"
                fi
                ;;
        esac

        KALAM_INSTALLER_REEXEC=1 exec bash -s -- "$@"
    fi

    echo "KalamDB installer requires bash. Install bash or run: curl -fsSL https://kalamdb.org/install.sh | bash" >&2
    exit 1
fi

set -euo pipefail

# ── Configurable ────────────────────────────────────────────────────────────
GITHUB_REPO="kalamdb/KalamDB"
BINARY_NAME="kalam"
ARTIFACT_PREFIX="kalamcli"
INSTALL_DIR="${KALAM_INSTALL_DIR:-$HOME/.kalam/bin}"
VERSION="${KALAM_VERSION:-}"
PRE_RELEASE="${KALAM_PRE_RELEASE:-0}"
NO_MODIFY_PATH="${KALAM_NO_MODIFY_PATH:-0}"

# ── Colors & helpers ────────────────────────────────────────────────────────
RED=$'\033[0;31m'
GREEN=$'\033[0;32m'
YELLOW=$'\033[1;33m'
BLUE=$'\033[0;34m'
BOLD=$'\033[1m'
NC=$'\033[0m' # No Color

info()  { printf "${BLUE}▸${NC} %s\n" "$*"; }
ok()    { printf "${GREEN}✔${NC} %s\n" "$*"; }
warn()  { printf "${YELLOW}⚠${NC} %s\n" "$*"; }
err()   { printf "${RED}✘${NC} %b\n" "$*" >&2; }
fatal() { err "$@"; exit 1; }

print_usage() {
    cat <<EOF
KalamDB CLI Installer

Usage:
  install.sh [--version <version>] [--pre-release] [--help]

Options:
  --version <version>  Install an exact version (for example 0.5.0 or v0.5.0)
  --pre-release        Install the latest GitHub prerelease
  --help               Show this help message

Environment variables:
  KALAM_INSTALL_DIR      Installation directory (default: $HOME/.kalam/bin)
  KALAM_VERSION          Specific version to install (same as --version)
  KALAM_PRE_RELEASE      Set to 1 to install the latest prerelease
  KALAM_NO_MODIFY_PATH   Set to 1 to skip PATH modification
EOF
}

parse_args() {
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --version)
                [[ $# -ge 2 ]] || fatal "Missing value for --version"
                VERSION="$2"
                shift 2
                ;;
            --version=*)
                VERSION="${1#*=}"
                shift
                ;;
            --pre-release)
                PRE_RELEASE="1"
                shift
                ;;
            --help|-h)
                print_usage
                exit 0
                ;;
            *)
                fatal "Unknown argument: $1\n\n$(print_usage)"
                ;;
        esac
    done
}

extract_tag_name() {
    sed -nE 's/.*"tag_name": *"([^"]+)".*/\1/p' | head -1
}

# ── Detect platform ────────────────────────────────────────────────────────
detect_platform() {
    local os arch

    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Linux)  os="linux" ;;
        Darwin) os="macos" ;;
        MINGW*|MSYS*|CYGWIN*) os="windows" ;;
        *) fatal "Unsupported operating system: $os" ;;
    esac

    case "$arch" in
        x86_64|amd64)  arch="x86_64" ;;
        aarch64|arm64) arch="aarch64" ;;
        *) fatal "Unsupported architecture: $arch" ;;
    esac

    PLATFORM="${os}-${arch}"
}

# ── Check required commands ─────────────────────────────────────────────────
check_deps() {
    local missing=()

    for cmd in curl tar; do
        if ! command -v "$cmd" &>/dev/null; then
            missing+=("$cmd")
        fi
    done

    if [[ ${#missing[@]} -gt 0 ]]; then
        fatal "Missing required commands: ${missing[*]}"
    fi
}

# ── Resolve latest version from GitHub ──────────────────────────────────────
resolve_version() {
    if [[ -n "$VERSION" ]]; then
        VERSION="${VERSION#v}"
        info "Using requested version: $VERSION"
        return
    fi

    local response=""
    local tag_name=""

    if [[ "$PRE_RELEASE" == "1" ]]; then
        info "Fetching latest prerelease version…"

        local prerelease_url="https://api.github.com/repos/${GITHUB_REPO}/releases?per_page=20"
        response="$(curl -fsSL "$prerelease_url" 2>/dev/null)" || {
            fatal "Could not reach GitHub API. Check your internet connection or set KALAM_VERSION."
        }

        tag_name="$(printf '%s\n' "$response" | awk '
            /"tag_name":/ {
                if (match($0, /"tag_name": *"[^"]+"/)) {
                    current_tag = substr($0, RSTART + 12, RLENGTH - 13)
                    gsub(/"/, "", current_tag)
                }
            }
            /"prerelease": true/ {
                if (current_tag != "") {
                    print current_tag
                    exit
                }
            }
        ')"

        if [[ -z "$tag_name" ]]; then
            fatal "Could not determine latest prerelease version. Set KALAM_VERSION explicitly."
        fi
    else
        info "Fetching latest release version…"

        # 1) Try the /releases/latest endpoint first
        local releases_url="https://api.github.com/repos/${GITHUB_REPO}/releases/latest"
        response="$(curl -fsSL "$releases_url" 2>/dev/null)" && \
            tag_name="$(printf '%s\n' "$response" | extract_tag_name)"

        # 2) Fall back to the first tag if no release exists yet
        if [[ -z "$tag_name" ]]; then
            local tags_url="https://api.github.com/repos/${GITHUB_REPO}/tags"
            response="$(curl -fsSL "$tags_url" 2>/dev/null)" || {
                fatal "Could not reach GitHub API. Check your internet connection or set KALAM_VERSION."
            }
            tag_name="$(echo "$response" | grep '"name"' | head -1 | sed -E 's/.*"name": *"([^"]+)".*/\1/')"
        fi
    fi

    if [[ -z "$tag_name" ]]; then
        fatal "Could not determine latest version. Set KALAM_VERSION explicitly (e.g. KALAM_VERSION=0.3.0-alpha2)."
    fi

    # Strip leading 'v' (artifact names don't include it)
    VERSION="${tag_name#v}"

    ok "Latest version: $VERSION"
}

# ── Download & install ──────────────────────────────────────────────────────
download_and_install() {
    # Windows releases use .zip; everything else uses .tar.gz
    local ext="tar.gz"
    if [[ "$PLATFORM" == windows-* ]]; then
        ext="zip"
    fi

    local archive="${ARTIFACT_PREFIX}-${VERSION}-${PLATFORM}.${ext}"
    local base_url="https://github.com/${GITHUB_REPO}/releases/download/v${VERSION}"
    local download_url="${base_url}/${archive}"
    local checksums_url="${base_url}/SHA256SUMS"

    local tmpdir=""
    tmpdir="$(mktemp -d)" || fatal "Could not create temporary directory"
    trap 'rm -rf "$tmpdir"' EXIT

    # Download the archive
    info "Downloading ${BOLD}${archive}${NC}…"
    curl -fsSL --progress-bar -o "${tmpdir}/${archive}" "$download_url" || {
        fatal "Download failed. Check that version '${VERSION}' exists at:\n  https://github.com/${GITHUB_REPO}/releases"
    }

    # Verify checksum if SHA256SUMS is available
    info "Verifying checksum…"
    if curl -fsSL -o "${tmpdir}/SHA256SUMS" "$checksums_url" 2>/dev/null; then
        local expected_hash actual_hash
        expected_hash="$(grep "${archive}" "${tmpdir}/SHA256SUMS" | awk '{print $1}')"

        if [[ -n "$expected_hash" ]]; then
            if command -v sha256sum &>/dev/null; then
                actual_hash="$(sha256sum "${tmpdir}/${archive}" | awk '{print $1}')"
            elif command -v shasum &>/dev/null; then
                actual_hash="$(shasum -a 256 "${tmpdir}/${archive}" | awk '{print $1}')"
            else
                warn "No sha256sum or shasum found — skipping checksum verification"
                actual_hash="$expected_hash"
            fi

            if [[ "$actual_hash" != "$expected_hash" ]]; then
                fatal "Checksum mismatch!\n  Expected: ${expected_hash}\n  Got:      ${actual_hash}"
            fi
            ok "Checksum verified"
        else
            warn "Checksum entry for ${archive} not found in SHA256SUMS — skipping verification"
        fi
    else
        warn "SHA256SUMS not available — skipping checksum verification"
    fi

    # Extract
    info "Extracting…"
    if [[ "$ext" == "zip" ]]; then
        command -v unzip &>/dev/null || fatal "unzip is required to install on Windows"
        unzip -q "${tmpdir}/${archive}" -d "${tmpdir}" || fatal "Failed to extract archive"
    else
        tar -xzf "${tmpdir}/${archive}" -C "${tmpdir}" || fatal "Failed to extract archive"
    fi

    # Find the binary — could be named 'kalam', 'kalamcli', with or without version suffix
    local binary_path
    binary_path="$(find "${tmpdir}" -type f \( -name "${BINARY_NAME}" -o -name "${ARTIFACT_PREFIX}" \) | head -1)"

    if [[ -z "$binary_path" ]]; then
        # Fallback: any executable file that isn't an archive or metadata
        binary_path="$(find "${tmpdir}" -type f ! -name '*.tar.gz' ! -name '*.zip' ! -name 'SHA256SUMS' ! -name '*.txt' ! -name '*.md' | head -1)"
    fi

    if [[ -z "$binary_path" ]]; then
        fatal "Could not find ${BINARY_NAME} binary in the archive"
    fi

    # Install as 'kalam' regardless of archive naming
    mkdir -p "$INSTALL_DIR"
    cp "$binary_path" "${INSTALL_DIR}/${BINARY_NAME}"
    chmod +x "${INSTALL_DIR}/${BINARY_NAME}"

    # Explicit cleanup and disarm the trap so 'set -u' doesn't complain after return
    rm -rf "$tmpdir"
    trap - EXIT

    ok "Installed ${BOLD}${BINARY_NAME}${NC} to ${INSTALL_DIR}/${BINARY_NAME}"
}

# ── Update PATH ─────────────────────────────────────────────────────────────
configure_path() {
    if [[ "$NO_MODIFY_PATH" == "1" ]]; then
        return
    fi

    # Check if already in PATH
    if echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
        return
    fi

    local export_line="export PATH=\"${INSTALL_DIR}:\$PATH\""
    local shell_name
    shell_name="$(basename "${SHELL:-/bin/bash}")"

    local rc_files=()

    case "$shell_name" in
        zsh)
            rc_files=("$HOME/.zshrc")
            ;;
        bash)
            # Prefer .bashrc on Linux, .bash_profile on macOS
            if [[ -f "$HOME/.bash_profile" ]]; then
                rc_files=("$HOME/.bash_profile")
            elif [[ -f "$HOME/.bashrc" ]]; then
                rc_files=("$HOME/.bashrc")
            else
                rc_files=("$HOME/.bashrc")
            fi
            ;;
        fish)
            local fish_conf="$HOME/.config/fish/config.fish"
            mkdir -p "$(dirname "$fish_conf")"
            if ! grep -qF "$INSTALL_DIR" "$fish_conf" 2>/dev/null; then
                echo "set -gx PATH \"${INSTALL_DIR}\" \$PATH" >> "$fish_conf"
                ok "Added ${INSTALL_DIR} to PATH in ${fish_conf}"
            fi
            return
            ;;
        *)
            rc_files=("$HOME/.profile")
            ;;
    esac

    for rc in "${rc_files[@]}"; do
        if ! grep -qF "$INSTALL_DIR" "$rc" 2>/dev/null; then
            echo "" >> "$rc"
            echo "# KalamDB CLI" >> "$rc"
            echo "$export_line" >> "$rc"
            ok "Added ${INSTALL_DIR} to PATH in ${rc}"
        fi
    done
}

# ── Main ────────────────────────────────────────────────────────────────────
main() {
    printf "\n${BOLD}  KalamDB CLI Installer${NC}\n\n"

    parse_args "$@"
    check_deps
    detect_platform
    info "Detected platform: ${BOLD}${PLATFORM}${NC}"

    resolve_version
    download_and_install
    configure_path

    printf "\n${GREEN}${BOLD}  Installation complete!${NC}\n\n"

    # Check if the binary is already on PATH
    if command -v "$BINARY_NAME" &>/dev/null; then
        local installed_path
        installed_path="$(command -v "$BINARY_NAME")"
        info "Binary available at: ${installed_path}"
        printf "\n  Run ${BOLD}kalam --help${NC} to get started.\n\n"
    else
        printf "  To get started, restart your shell or run:\n\n"
        printf "    ${BOLD}export PATH=\"${INSTALL_DIR}:\$PATH\"${NC}\n\n"
        printf "  Then run ${BOLD}kalam --help${NC}\n\n"
    fi
}

main "$@"
