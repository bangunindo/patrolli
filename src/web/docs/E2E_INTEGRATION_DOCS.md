# Web E2E Stack & GitHub Actions CI Guide

This guide is intended for DevOps and engineers to run and integrate frontend E2E testing (`src/web`) into GitHub Actions CI as well as on a new local machine.

---

## 1. E2E Stack Architecture

The Web E2E suite (`npm run test:e2e`) tests the Nuxt SSR + Nitro BFF application end-to-end against real backend services:

| Service      | Image / Runtime                                             | Host Port         | Role                                                                          |
| ------------ | ----------------------------------------------------------- | ----------------- | ----------------------------------------------------------------------------- |
| `postgres`   | `postgres:18-alpine`                                        | `127.0.0.1:5432`  | Database for `patrolli` (`patrolli`, `patrolli1`) and `keycloak` (`keycloak`) |
| `keycloak`   | `quay.io/keycloak/keycloak:26.7.1`                          | `127.0.0.1:8443`  | OIDC SSO Identity Provider (`https://kc.localtest.me:8443/realms/patrolli`)   |
| `patrolli`   | `${PATROLLI_CORE_IMAGE}`                                    | `127.0.0.1:8080`  | Primary Patrolli Core cluster (local auth + OIDC)                             |
| `patrolli_1` | `${PATROLLI_CORE_IMAGE}`                                    | `127.0.0.1:30001` | Secondary cluster for multi-cluster switching tests                           |
| `frontend`   | Playwright `webServer` (`npm run build && npm run preview`) | `3000`            | Nuxt/Nitro BFF server (`https://localhost:3000`)                              |

---

## 2. `src/web/dev/` Directory Structure

```text
src/web/dev/
├── .dockerignore
├── docker-compose.yml               # E2E backend stack definition (uses named volume postgres-data)
├── init.sql                         # Initializes keycloak & patrolli1 DBs on first Postgres creation
├── setup.sh                         # Main script: generates certs, ephemeral secrets, and renders templates
├── ssl.sh                           # Root CA + server certs generator (frontend & keycloak)
├── certs/
│   ├── root.cnf                     # Root CA configuration
│   ├── frontend.cnf                 # SAN: localhost, 127.0.0.1
│   ├── keycloak.cnf                 # SAN: kc.localtest.me, localhost, 127.0.0.1
│   ├── csr/                         # (Gitignored) Temporary CSRs
│   └── ssl/                         # (Gitignored) Generated *.crt, *.key, *.srl files
├── config/
│   ├── backend/
│   │   ├── patrolli-0-config.template.yaml
│   │   └── patrolli-1-config.template.yaml
│   ├── frontend/
│   │   └── config-fe.template.yaml
│   └── keycloak/
│       └── realm-config.template.json
├── .env                             # (Gitignored) Ephemeral secrets for docker compose
└── .generated/                      # (Gitignored) Rendered runtime configs
```

---

## 3. Setup Mechanism & Secret Security

No TLS private keys or static secrets are stored in Git:

1. **`./dev/ssl.sh`**:
    - Generates the CA, `frontend.crt/.key`, and `keycloak.crt/.key` using `openssl`.
    - Sets `chmod 600` on `ssl/ca.key` & `ssl/frontend.key`, and `chmod 644` on `ssl/*.crt` & `ssl/keycloak.key` so non-root containers (such as Keycloak `uid 1000` on Linux runners) can read keys mounted `:ro`.
2. **`./dev/setup.sh`**:
    - Runs `ssl.sh`.
    - Generates random ephemeral secrets (`E2E_JWT_SECRET`, `E2E_OIDC_SECRET`, `E2E_SECRET_KEY`, `E2E_SECRET_KEY_1`, `E2E_POSTGRES_PASSWORD`, `E2E_KC_BOOTSTRAP_PASSWORD`, `E2E_ADMIN_PASSWORD`) and writes them to `dev/.env` (`chmod 600`). All values can be overridden via environment variables before running the script.
    - Renders all `*.template.*` files into `dev/.generated/` (with `chmod 600` on `config-fe.yaml`) and copies `config-fe.yaml` to `src/web/config-fe.yaml` (if it does not exist yet, or always overwrites when passed the `--force` flag).
    - If `GITHUB_ENV` is active (in GitHub Actions), masks passwords (`::add-mask::`) and exports `E2E_USERNAME=admin` and `E2E_PASSWORD=${E2E_ADMIN_PASSWORD}`.
3. **`playwright.config.ts`**:
    - Locally, if `E2E_PASSWORD` is not already set in the shell, `playwright.config.ts` reads only `E2E_ADMIN_PASSWORD` from `dev/.env` (using `node:util` `parseEnv`) without loading other backend secrets into the Playwright environment.
    - The frontend config file can be pointed via the `FRONTEND_CONFIG` env var (or `CONFIG_PATH` / `CONFIG_FILE`), falling back to `src/web/config-fe.yaml` by default.

---

## 4. Running E2E on a New Machine (Local)

Prerequisites: **Docker & Docker Compose**, **Node.js 24**, **OpenSSL**.

```bash
cd src/web

# 1. Install dependencies & Playwright browsers
npm ci
npx playwright install chromium --with-deps

# 2. Generate TLS certificates & ephemeral configs
./dev/setup.sh --force

# 3. Start the backend stack & wait until healthy
docker compose -f dev/docker-compose.yml up -d --wait

# 4. Run E2E tests
npm run test:e2e

# 5. Clean up when finished (optional: -v removes the Postgres volume)
docker compose -f dev/docker-compose.yml down -v
```

> [!NOTE]
> If you are offline or public DNS does not resolve `*.localtest.me`, add `127.0.0.1 kc.localtest.me` to `/etc/hosts`.

---

## 5. GitHub Actions CI Integration Guide

### A. `patrolli-core` Image Strategy (Without Compiling Rust on Web PRs)

In `dev/docker-compose.yml`, the backend image uses:

```yaml
image: ${PATROLLI_CORE_IMAGE:-ghcr.io/bangunindo/patrolli-core:0.1.0-beta.3}
pull_policy: ${PATROLLI_CORE_PULL_POLICY:-always}
```

Recommended CI workflow:

1. **Publish the `:edge` tag on `push` to `main`** (when `src/core/**` changes):
    - Build the image from `src/core/Dockerfile` (`linux/amd64`) and push it to `ghcr.io/bangunindo/patrolli-core:edge`.
    - Must maintain **AGPL-3.0** compliance as in `.github/workflows/build.yaml`:
        - Labels `org.opencontainers.image.licenses=AGPL-3.0-only` & `org.opencontainers.image.source=https://github.com/bangunindo/patrolli`.
        - `provenance: mode=max`, `sbom: true`, and `actions/attest-build-provenance`.
        - The GHCR package must have **public read** access so PRs from external forks can still `docker pull` without secrets.
2. **When a `pull_request` runs**:
    - **If only `src/web/` changed**: pull `ghcr.io/bangunindo/patrolli-core:edge` directly (no Rust build). `pull_policy: always` ensures the latest image tag is always pulled.
    - **If `src/core/` also changed**: build a local image from `src/core` (`docker build -t patrolli-core:pr-local src/core`), then set `PATROLLI_CORE_IMAGE=patrolli-core:pr-local` and `PATROLLI_CORE_PULL_POLICY=never`.

### B. `web-e2e` Job Reference for `.github/workflows/test.yaml`

Per this repository's security standards, **pin all GitHub Actions to a commit SHA** and set `persist-credentials: false`:

```yaml
web-e2e:
    if: inputs.web
    runs-on: ubuntu-24.04
    timeout-minutes: 30
    permissions:
        contents: read
        packages: read
    defaults:
        run:
            working-directory: src/web
    steps:
        - uses: actions/checkout@<COMMIT_SHA>
          with:
              persist-credentials: false

        - uses: actions/setup-node@<COMMIT_SHA>
          with:
              node-version: 24
              cache: npm
              cache-dependency-path: src/web/package-lock.json

        - name: Map kc.localtest.me to localhost
          run: echo "127.0.0.1 kc.localtest.me" | sudo tee -a /etc/hosts

        - name: Generate ephemeral certs and E2E configs
          run: ./dev/setup.sh --force

        - name: Start backend stack
          env:
              PATROLLI_CORE_IMAGE: ghcr.io/bangunindo/patrolli-core:edge
          run: |
              docker compose -f dev/docker-compose.yml up -d --wait
              for _ in $(seq 1 45); do
                if curl -sf http://localhost:8080/health >/dev/null && \
                   curl -sf http://localhost:30001/health >/dev/null && \
                   curl --cacert dev/certs/ssl/ca.crt -sf https://kc.localtest.me:8443/realms/patrolli >/dev/null; then
                  exit 0
                fi
                sleep 2
              done
              docker compose -f dev/docker-compose.yml logs
              exit 1

        - run: npm ci

        - name: Cache Playwright browsers
          id: playwright-cache
          uses: actions/cache@<COMMIT_SHA>
          with:
              path: ~/.cache/ms-playwright
              key: playwright-${{ runner.os }}-${{ hashFiles('src/web/package-lock.json') }}

        - name: Install Playwright Chromium
          run: |
              if [ "${{ steps.playwright-cache.outputs.cache-hit }}" != "true" ]; then
                npx playwright install chromium --with-deps
              else
                npx playwright install-deps chromium
              fi

        - name: Run Web E2E
          env:
              FRONTEND_CONFIG: dev/.generated/config-fe.yaml
          run: npm run test:e2e

        - name: Upload Playwright report on failure
          if: failure()
          uses: actions/upload-artifact@<COMMIT_SHA>
          with:
              name: playwright-report
              path: |
                  src/web/playwright-report
                  src/web/test-results
              retention-days: 7

        - name: Stop backend stack
          if: always()
          run: docker compose -f dev/docker-compose.yml down -v
```

---

## 6. License Compliance, Linting, & Docker Build Isolation

- **AGPL-3.0 / REUSE (`fsfe/reuse-action`)**:
    - Every new file in `src/web/dev/` must have `SPDX-FileCopyrightText` & `SPDX-License-Identifier` (`AGPL-3.0-only`) headers, or be covered in `/REUSE.toml` (such as `**/*.json`, `**/*.md`, `src/web/dev/certs/*.cnf`, and `src/web/dev/.dockerignore`).
- **Prettier & ESLint (`npm run lint`)**:
    - `src/web/.prettierignore` ignores `dev` so Compose/YAML/JSON config files do not trigger formatting errors when running `prettier --check .`.
- **Docker Build Hygiene (`src/web/.dockerignore`)**:
    - `.env`, `.env.*`, `config-fe.yaml`, `config-fe.yml`, and `dev` are included in `src/web/.dockerignore` so local E2E artifacts are not bundled into the production `src/web/Dockerfile` image build.

---

## 7. Troubleshooting

- **Keycloak fails to read `/opt/keycloak/conf/keycloak.key` (`Permission denied`)**:
    - Re-run `./dev/ssl.sh` or ensure the key permission is `644`.
- **Keycloak SSO setup times out on the _"Update Account Information"_ page**:
    - Users in `realm-config.template.json` must have `email`, `firstName`, `lastName`, and `"requiredActions": []`. If you modify the template after the DB has already been initialized, reset the volume first with `docker compose -f dev/docker-compose.yml down -v`.
- **Login setup fails with `401 Invalid credentials`**:
    - Ensure the containers are started after running `./dev/setup.sh` so `E2E_ADMIN_PASSWORD` in `dev/.env` matches what `playwright.config.ts` reads.
- **Pipeline test fails with _"At least one host must be assigned"_**:
    - Ensure `pipeline-builder.spec.ts` and `pipeline-management.spec.ts` use the `apiHelper.createHost(...)` helper (already implemented to run cleanly on an empty DB).
