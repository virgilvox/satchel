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

# `embed-model` reads vault/models/bge-small-en-v1.5/* via include_bytes!
# at compile time. CI does this in release.yml; local builds need the model
# already on disk. Bail loudly so users don't ship binaries that silently
# fall back to the "Unavailable" embedder.
MODEL_DIR="${PROJECT_DIR}/vault/models/bge-small-en-v1.5"
for f in model.safetensors tokenizer.json config.json; do
    if [[ ! -f "${MODEL_DIR}/${f}" ]]; then
        echo "error: ${MODEL_DIR}/${f} missing — run scripts/download-model.sh first" >&2
        exit 1
    fi
done

mkdir -p "${OUT_DIR}"

for target in "${TARGETS[@]}"; do
    echo "=== Building for ${target} ==="

    # Install target if needed
    rustup target add "${target}" 2>/dev/null || true

    cargo build --release --features embed-model --target "${target}" \
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
