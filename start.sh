#!/usr/bin/env bash
# SATCHEL: Portable RAG system launcher.
# Auto-detects the platform and runs the correct binary.
# Place this script at the root of the USB drive.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
VAULT_DIR="${SCRIPT_DIR}/vault"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"

case "${ARCH}" in
    x86_64|amd64) ARCH="x86_64" ;;
    aarch64|arm64) ARCH="aarch64" ;;
    *) echo "[satchel] Unsupported architecture: ${ARCH}"; exit 1 ;;
esac

case "${OS}" in
    linux)   BINARY="${SCRIPT_DIR}/bin/satchel-linux-${ARCH}" ;;
    darwin)  BINARY="${SCRIPT_DIR}/bin/satchel-macos-${ARCH}" ;;
    mingw*|msys*|cygwin*) BINARY="${SCRIPT_DIR}/bin/satchel-windows-${ARCH}.exe" ;;
    *) echo "[satchel] Unsupported OS: ${OS}"; exit 1 ;;
esac

if [ ! -f "${BINARY}" ]; then
    echo "[satchel] Binary not found: ${BINARY}"
    echo "[satchel] Build with: cargo build --release"
    exit 1
fi

chmod +x "${BINARY}" 2>/dev/null || true

echo "[satchel] Platform: ${OS}/${ARCH}" >&2
echo "[satchel] Vault:    ${VAULT_DIR}" >&2

exec "${BINARY}" --vault "${VAULT_DIR}" "$@"
