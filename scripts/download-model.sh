#!/usr/bin/env bash
# Download the embedding model for offline use.
# Run this once on a machine with internet access.
#
# Default model: BAAI/bge-small-en-v1.5 (33M params, 384-d, MTEB ~62).
# Pass `legacy` as the first argument to fetch the older
# sentence-transformers/all-MiniLM-L6-v2 (22M params, 384-d, MTEB ~57)
# instead — useful for vaults already indexed under the old model.

set -euo pipefail

MODE="${1:-}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

if [ "$MODE" = "legacy" ]; then
    MODEL_NAME="all-MiniLM-L6-v2"
    BASE_URL="https://huggingface.co/sentence-transformers/${MODEL_NAME}/resolve/main"
else
    MODEL_NAME="bge-small-en-v1.5"
    BASE_URL="https://huggingface.co/BAAI/${MODEL_NAME}/resolve/main"
fi

MODEL_DIR="${SCRIPT_DIR}/../vault/models/${MODEL_NAME}"
mkdir -p "${MODEL_DIR}"

echo "[satchel] Downloading ${MODEL_NAME}..."

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
