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

Node.js 22 with npm 9 does not work: `npm ci` fails with `EBADPLATFORM` on an optional `@oxc-parser` binding. Ports `5432`, `8443`, `8080`, `30001` and `3000` must be free on localhost.

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

To test against the core of your checkout instead of the released image:

```bash
docker build -t patrolli-core:local ../core
PATROLLI_CORE_IMAGE=patrolli-core:local PATROLLI_CORE_PULL_POLICY=never \
    docker compose -f dev/docker-compose.yml up -d --wait
```

---

## 5. GitHub Actions CI Integration

The suite runs from the reusable workflow `.github/workflows/web-e2e.yaml`. It has two callers:

| Caller                 | When                                              | `patrolli-core` image under test                              | Effect of a failure                     |
| ---------------------- | ------------------------------------------------- | ------------------------------------------------------------- | --------------------------------------- |
| `ci.yaml`              | Pull requests that touch `src/web/` or `.github/` | Core of the PR's `src/core` tree (normally the one on `main`) | `ci-ok` fails, the PR cannot merge      |
| `build.yaml` (release) | `v<semver>` tags                                  | The `linux/amd64` core image built for that tag               | No version or `latest` tag is published |

It does **not** run for core-only pull requests or on pushes to `main`. Core and web have different owners and core is allowed to break web between releases; the release run is where both must agree.

### A. Which core image a pull request uses

Pushes to `main` run the `core-image` job in `ci.yaml`, which builds `src/core` (`linux/amd64`) and pushes it to GHCR only, tagged by the Git tree hash of `src/core`:

```bash
ghcr.io/bangunindo/patrolli-core:tree-$(git rev-parse HEAD:src/core)
```

The tree hash only changes when something under `src/core` changes, so:

- A push to `main` that does not change core finds its tag already published and skips the build.
- A web-only pull request has the same `src/core` tree as `main`, so `web-e2e.yaml` finds the tag and pulls it. No Rust build.
- If the tag is missing (the pull request also changes core, the `main` build has not finished yet, or nothing was published yet), `web-e2e.yaml` builds `src/core` from source in the job and runs against that. This is slow but always correct, and needs no secrets, so it also works for pull requests from forks.

The GHCR package must stay **public** so forks can pull without logging in. `tree-*` tags accumulate; clean them up with a retention rule on the package.

### B. Release gate

In `build.yaml` the per-arch images are pushed by digest first, without tags. The `web-e2e` job then runs web at that tag against `ghcr.io/bangunindo/patrolli-core@sha256:<linux/amd64 digest>` (taken from the `digests-core-amd64` artifact). The `merge` job, which creates the version tags, depends on it.

### C. What the job does

1. Sets up Node 24 and maps `kc.localtest.me` to `127.0.0.1` in `/etc/hosts`.
2. Resolves the core image as described above and exports `PATROLLI_CORE_IMAGE` and `PATROLLI_CORE_PULL_POLICY` for Compose.
3. Runs `npm ci` and `./dev/setup.sh --force`.
4. Starts the stack with `docker compose up -d --wait`, then polls the two core `/health` endpoints and the Keycloak realm. Only Postgres has a healthcheck, so `--wait` alone returns too early.
5. Installs Chromium with `npx playwright install chromium --with-deps` and runs `npm run test:e2e` with `FRONTEND_CONFIG=dev/.generated/config-fe.yaml`. `CI=true` makes Playwright use one worker and two retries.
6. On failure, prints the backend logs and uploads `test-results` (traces and screenshots) as the `playwright-report` artifact, kept for 7 days.

Reference timings on a 22-core machine with nothing cached: `npm ci` 45s, Chromium download 25s, stack start 45s, e2e run 3 minutes (including a 20s Nuxt build).

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
