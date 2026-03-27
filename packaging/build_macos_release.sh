#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
RELEASE_NAME="CodexCliVoiceBridgeRust_macOS"
RELEASE_ROOT="${PROJECT_ROOT}/packaging/release"
OUTPUT_DIR="${RELEASE_ROOT}/${RELEASE_NAME}"
ARCHIVE_PATH="${RELEASE_ROOT}/${RELEASE_NAME}.tar.gz"

echo "[BUILD] project root: ${PROJECT_ROOT}"
echo "[BUILD] release dir:  ${OUTPUT_DIR}"

cd "${PROJECT_ROOT}"
cargo build --release --bin voice_bridge --bin doubao_asr_demo --bin configure_credentials --bin voice_bridge_doctor

rm -rf "${OUTPUT_DIR}"
mkdir -p "${OUTPUT_DIR}"

RELEASE_BIN_DIR="${PROJECT_ROOT}/target/release"

cp "${RELEASE_BIN_DIR}/voice_bridge" "${OUTPUT_DIR}/voice_bridge"
cp "${RELEASE_BIN_DIR}/doubao_asr_demo" "${OUTPUT_DIR}/doubao_asr_demo"
cp "${RELEASE_BIN_DIR}/configure_credentials" "${OUTPUT_DIR}/configure_credentials"
cp "${RELEASE_BIN_DIR}/voice_bridge_doctor" "${OUTPUT_DIR}/voice_bridge_doctor"
cp "${PROJECT_ROOT}/doubao_credentials.example.json" "${OUTPUT_DIR}/doubao_credentials.example.json"
cp "${PROJECT_ROOT}/packaging/README_macos_release.md" "${OUTPUT_DIR}/README_macos_release.md"

cat > "${OUTPUT_DIR}/doubao_credentials.json" <<'EOF'
{
  "app_id": "",
  "access_token": ""
}
EOF

cat > "${OUTPUT_DIR}/voice_bridge.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "${SCRIPT_DIR}"
./voice_bridge "$@"
EOF

cat > "${OUTPUT_DIR}/start_voice_bridge.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "${SCRIPT_DIR}"
./voice_bridge "$@"
EOF

cat > "${OUTPUT_DIR}/doubao_asr_demo.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "${SCRIPT_DIR}"
./doubao_asr_demo "$@"
EOF

cat > "${OUTPUT_DIR}/configure_credentials.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "${SCRIPT_DIR}"
./configure_credentials "$@"
EOF

cat > "${OUTPUT_DIR}/voice_bridge_doctor.sh" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "${SCRIPT_DIR}"
./voice_bridge_doctor "$@"
EOF

chmod +x \
  "${OUTPUT_DIR}/voice_bridge" \
  "${OUTPUT_DIR}/doubao_asr_demo" \
  "${OUTPUT_DIR}/configure_credentials" \
  "${OUTPUT_DIR}/voice_bridge_doctor" \
  "${OUTPUT_DIR}/voice_bridge.sh" \
  "${OUTPUT_DIR}/start_voice_bridge.sh" \
  "${OUTPUT_DIR}/doubao_asr_demo.sh" \
  "${OUTPUT_DIR}/configure_credentials.sh" \
  "${OUTPUT_DIR}/voice_bridge_doctor.sh"

rm -f "${ARCHIVE_PATH}"
tar -C "${RELEASE_ROOT}" -czf "${ARCHIVE_PATH}" "${RELEASE_NAME}"

echo "[DONE] output dir: ${OUTPUT_DIR}"
echo "[DONE] archive:    ${ARCHIVE_PATH}"
