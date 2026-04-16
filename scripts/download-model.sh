#!/usr/bin/env bash
# Download the embedding model for offline use.
# Run this once on a machine with internet access.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
MODEL_DIR="${SCRIPT_DIR}/../vault/models/all-MiniLM-L6-v2"

mkdir -p "${MODEL_DIR}"

echo "[satchel] Downloading all-MiniLM-L6-v2 model..."

BASE_URL="https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/main"

for FILE in tokenizer.json config.json model.safetensors; do
    if [ ! -f "${MODEL_DIR}/${FILE}" ]; then
        echo "[satchel] Downloading ${FILE}..."
        curl -L -o "${MODEL_DIR}/${FILE}" "${BASE_URL}/${FILE}"
    else
        echo "[satchel] ${FILE} already exists"
    fi
done

echo ""
echo "[satchel] Model downloaded to: ${MODEL_DIR}"
echo "[satchel] Size: $(du -sh "${MODEL_DIR}" | cut -f1)"
echo "[satchel] Setup complete. No additional runtime libraries needed."
