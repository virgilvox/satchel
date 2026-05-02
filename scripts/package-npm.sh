#!/usr/bin/env bash
# Generate npm platform packages from release binaries.
# Run after building all platform binaries.
#
# Usage:
#   ./scripts/package-npm.sh <release-dir>
#
# The release-dir should contain:
#   satchel-linux-x86_64
#   satchel-linux-aarch64
#   satchel-macos-x86_64
#   satchel-macos-aarch64
#   satchel-windows-x86_64.exe

set -euo pipefail

RELEASE_DIR="${1:?Usage: package-npm.sh <release-dir>}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
NPM_DIR="${SCRIPT_DIR}/../npm"
VERSION=$(grep '^version' "${SCRIPT_DIR}/../Cargo.toml" | head -1 | sed 's/.*"\(.*\)".*/\1/')

echo "[npm] Packaging version ${VERSION}"

declare -A PLATFORM_MAP=(
    ["darwin-arm64"]="satchel-macos-aarch64"
    ["darwin-x64"]="satchel-macos-x86_64"
    ["linux-arm64"]="satchel-linux-aarch64"
    ["linux-x64"]="satchel-linux-x86_64"
    ["win32-x64"]="satchel-windows-x86_64.exe"
)

declare -A OS_MAP=(
    ["darwin-arm64"]="darwin"
    ["darwin-x64"]="darwin"
    ["linux-arm64"]="linux"
    ["linux-x64"]="linux"
    ["win32-x64"]="win32"
)

declare -A CPU_MAP=(
    ["darwin-arm64"]="arm64"
    ["darwin-x64"]="x64"
    ["linux-arm64"]="arm64"
    ["linux-x64"]="x64"
    ["win32-x64"]="x64"
)

for platform in "${!PLATFORM_MAP[@]}"; do
    binary="${PLATFORM_MAP[$platform]}"
    os_val="${OS_MAP[$platform]}"
    cpu_val="${CPU_MAP[$platform]}"
    pkg_name="@satchel-rag/${platform}"
    pkg_dir="${NPM_DIR}/${platform}"

    echo "[npm] Creating ${pkg_name}"
    mkdir -p "${pkg_dir}"

    # Determine binary name in package
    if [[ "${platform}" == win32-* ]]; then
        bin_name="satchel.exe"
    else
        bin_name="satchel"
    fi

    # Copy binary
    if [ -f "${RELEASE_DIR}/${binary}" ]; then
        cp "${RELEASE_DIR}/${binary}" "${pkg_dir}/${bin_name}"
        chmod +x "${pkg_dir}/${bin_name}" 2>/dev/null || true
    else
        echo "[npm] WARNING: Binary not found: ${RELEASE_DIR}/${binary}"
        continue
    fi

    # Generate package.json
    cat > "${pkg_dir}/package.json" << PKGJSON
{
  "name": "${pkg_name}",
  "version": "${VERSION}",
  "description": "SATCHEL binary for ${platform}",
  "license": "MIT",
  "repository": {
    "type": "git",
    "url": "https://github.com/virgilvox/satchel"
  },
  "os": ["${os_val}"],
  "cpu": ["${cpu_val}"],
  "files": ["${bin_name}"]
}
PKGJSON

    echo "[npm] Created ${pkg_dir}"
done

# Update root package version
sed -i.bak "s/\"version\": \".*\"/\"version\": \"${VERSION}\"/" "${NPM_DIR}/satchel/package.json"
rm -f "${NPM_DIR}/satchel/package.json.bak"

echo ""
echo "[npm] Packages ready in ${NPM_DIR}/"
echo "[npm] To publish: cd npm/<pkg> && npm publish --access public"
