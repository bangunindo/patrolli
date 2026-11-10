#!/usr/bin/env bash
# SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
#
# SPDX-License-Identifier: AGPL-3.0-only

set -euo pipefail

CYAN='\033[0;36m'
GREEN='\033[0;32m'
NC='\033[0m'

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CERTS_DIR="${SCRIPT_DIR}/certs"

mkdir -p "${CERTS_DIR}/ssl" "${CERTS_DIR}/csr"
cd "${CERTS_DIR}"

echo -e "${CYAN}=== 1. Creating single Root CA ===${NC}"
openssl req -x509 -new -nodes -days 3650 \
	-keyout ssl/ca.key -out ssl/ca.crt \
	-config root.cnf

echo -e "${CYAN}=== 2. Creating Server Certificate (frontend) ===${NC}"
openssl genrsa -out ssl/frontend.key 2048
openssl req -new -key ssl/frontend.key -out csr/frontend.csr -config frontend.cnf

echo -e "${CYAN}=== 3. Creating Server Certificate (keycloak) ===${NC}"
openssl genrsa -out ssl/keycloak.key 2048
openssl req -new -key ssl/keycloak.key -out csr/keycloak.csr -config keycloak.cnf

echo -e "${CYAN}=== 4. Signing CSRs with Root CA ===${NC}"
openssl x509 -req -in csr/frontend.csr \
	-CA ssl/ca.crt \
	-CAkey ssl/ca.key \
	-CAcreateserial \
	-out ssl/frontend.crt \
	-days 365 \
	-sha256 \
	-extfile frontend.cnf \
	-extensions v3_req

openssl x509 -req -in csr/keycloak.csr \
	-CA ssl/ca.crt \
	-CAkey ssl/ca.key \
	-CAcreateserial \
	-out ssl/keycloak.crt \
	-days 365 \
	-sha256 \
	-extfile keycloak.cnf \
	-extensions v3_req

echo -e "${CYAN}=== 5. Verifying Certificates ===${NC}"
openssl verify -CAfile ssl/ca.crt ssl/frontend.crt
openssl verify -CAfile ssl/ca.crt ssl/keycloak.crt

# Restrict host-only private keys; keep certs and keycloak.key readable by non-root containers
chmod 600 ssl/ca.key ssl/frontend.key
chmod 644 ssl/*.crt ssl/keycloak.key

echo -e "${GREEN}=== Done! All certificates are ready in ${CERTS_DIR}/ssl ===${NC}"
