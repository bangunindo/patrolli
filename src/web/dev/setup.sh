#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
#
# SPDX-License-Identifier: AGPL-3.0-only

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WEB_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
GEN_DIR="${SCRIPT_DIR}/.generated"
FORCE=false

for arg in "$@"; do
	case "$arg" in
		--force|-f) FORCE=true ;;
		*)
			echo "Unknown argument: $arg" >&2
			exit 1
			;;
	esac
done

mkdir -p "${GEN_DIR}"

# 1. Generate ephemeral TLS certificates
"${SCRIPT_DIR}/ssl.sh"

# 2. Generate ephemeral secrets (respecting overrides if already exported)
E2E_JWT_SECRET="${E2E_JWT_SECRET:-$(openssl rand -hex 32)}"
E2E_OIDC_SECRET="${E2E_OIDC_SECRET:-$(openssl rand -hex 32)}"
E2E_SECRET_KEY="${E2E_SECRET_KEY:-v1:$(openssl rand -base64 32)}"
E2E_SECRET_KEY_1="${E2E_SECRET_KEY_1:-v1:$(openssl rand -base64 32)}"
E2E_POSTGRES_PASSWORD="${E2E_POSTGRES_PASSWORD:-$(openssl rand -hex 16)}"
E2E_KC_BOOTSTRAP_PASSWORD="${E2E_KC_BOOTSTRAP_PASSWORD:-$(openssl rand -hex 16)}"
E2E_ADMIN_PASSWORD="${E2E_PASSWORD:-${E2E_ADMIN_PASSWORD:-$(openssl rand -hex 16)}}"

# 3. Write .env for docker compose
cat > "${SCRIPT_DIR}/.env" <<EOF
E2E_JWT_SECRET=${E2E_JWT_SECRET}
E2E_SECRET_KEY=${E2E_SECRET_KEY}
E2E_SECRET_KEY_1=${E2E_SECRET_KEY_1}
E2E_POSTGRES_PASSWORD=${E2E_POSTGRES_PASSWORD}
E2E_KC_BOOTSTRAP_PASSWORD=${E2E_KC_BOOTSTRAP_PASSWORD}
E2E_ADMIN_PASSWORD=${E2E_ADMIN_PASSWORD}
EOF
chmod 600 "${SCRIPT_DIR}/.env"

# 4. Render templates safely using Node.js (always available in src/web)
render_template() {
	local src="$1"
	local dst="$2"
	E2E_OIDC_SECRET="${E2E_OIDC_SECRET}" \
	E2E_ADMIN_PASSWORD="${E2E_ADMIN_PASSWORD}" \
	E2E_POSTGRES_PASSWORD="${E2E_POSTGRES_PASSWORD}" \
	node -e '
		const fs = require("node:fs");
		let text = fs.readFileSync(process.argv[1], "utf8");
		text = text
			.replaceAll("__E2E_OIDC_SECRET__", process.env.E2E_OIDC_SECRET)
			.replaceAll("__E2E_ADMIN_PASSWORD__", process.env.E2E_ADMIN_PASSWORD)
			.replaceAll("__E2E_POSTGRES_PASSWORD__", process.env.E2E_POSTGRES_PASSWORD);
		fs.writeFileSync(process.argv[2], text, "utf8");
	' "$src" "$dst"
}

render_template \
	"${SCRIPT_DIR}/config/keycloak/realm-config.template.json" \
	"${GEN_DIR}/realm-config.json"

render_template \
	"${SCRIPT_DIR}/config/backend/patrolli-0-config.template.yaml" \
	"${GEN_DIR}/patrolli-0-config.yaml"

render_template \
	"${SCRIPT_DIR}/config/backend/patrolli-1-config.template.yaml" \
	"${GEN_DIR}/patrolli-1-config.yaml"

render_template \
	"${SCRIPT_DIR}/config/frontend/config-fe.template.yaml" \
	"${GEN_DIR}/config-fe.yaml"

chmod 600 "${GEN_DIR}/config-fe.yaml"
chmod 644 "${GEN_DIR}/realm-config.json" "${GEN_DIR}/patrolli-0-config.yaml" "${GEN_DIR}/patrolli-1-config.yaml"

# Copy config-fe.yaml to src/web/config-fe.yaml if missing or --force
if [[ ! -f "${WEB_DIR}/config-fe.yaml" || "${FORCE}" == "true" ]]; then
	cp "${GEN_DIR}/config-fe.yaml" "${WEB_DIR}/config-fe.yaml"
	chmod 600 "${WEB_DIR}/config-fe.yaml"
else
	echo "Note: ${WEB_DIR}/config-fe.yaml already exists; rendered copy saved at ${GEN_DIR}/config-fe.yaml (use --force to overwrite)."
fi

# Export credentials for Playwright in GitHub Actions
if [[ -n "${GITHUB_ENV:-}" ]]; then
	echo "::add-mask::${E2E_ADMIN_PASSWORD}"
	{
		echo "E2E_USERNAME=admin"
		echo "E2E_PASSWORD=${E2E_ADMIN_PASSWORD}"
	} >> "${GITHUB_ENV}"
fi

echo "E2E environment generated in ${GEN_DIR}"
