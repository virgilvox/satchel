#!/usr/bin/env bash
# Build SATCHEL for all supported platforms
# Produces static binaries that run without any runtime dependencies

set -euo pipefail

TARGETS=(
    "x86_64-unknown-linux-musl"
    "aarch64-unknown-linux-musl"
    "x86_64-apple-darwin"
    "aarch64-apple-darwin"
    "x86_64-pc-windows-gnu"
)

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="${SCRIPT_DIR}/.."
OUT_DIR="${PROJECT_DIR}/bin"

mkdir -p "${OUT_DIR}"

for target in "${TARGETS[@]}"; do
    echo "=== Building for ${target} ==="

    # Install target if needed
    rustup target add "${target}" 2>/dev/null || true

    cargo build --release --target "${target}" \
        --manifest-path "${PROJECT_DIR}/Cargo.toml"

    # Copy binary with platform-friendly name
    case "${target}" in
        *linux*)
            arch="${target%%-*}"
            cp "target/${target}/release/satchel" "${OUT_DIR}/satchel-linux-${arch}"
            ;;
        *apple*)
            arch="${target%%-*}"
            cp "target/${target}/release/satchel" "${OUT_DIR}/satchel-macos-${arch}"
            ;;
        *windows*)
            arch="${target%%-*}"
            cp "target/${target}/release/satchel.exe" "${OUT_DIR}/satchel-windows-${arch}.exe"
            ;;
    esac

    echo "  Done: ${target}"
done

echo ""
echo "Binaries in: ${OUT_DIR}/"
ls -lh "${OUT_DIR}/"
