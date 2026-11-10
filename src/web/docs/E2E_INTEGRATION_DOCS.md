# Web E2E Stack & GitHub Actions CI Guide

Panduan ini ditujukan bagi DevOps dan engineer untuk menjalankan serta mengintegrasikan E2E testing frontend (`src/web`) ke GitHub Actions CI maupun di device baru.

---

## 1. Arsitektur Stack E2E

Suite E2E Web (`npm run test:e2e`) menguji aplikasi Nuxt SSR + Nitro BFF secara end-to-end terhadap layanan backend nyata:

| Service      | Image / Runtime                                             | Port Host         | Peran                                                                           |
| ------------ | ----------------------------------------------------------- | ----------------- | ------------------------------------------------------------------------------- |
| `postgres`   | `postgres:18-alpine`                                        | `127.0.0.1:5432`  | Database untuk `patrolli` (`patrolli`, `patrolli1`) dan `keycloak` (`keycloak`) |
| `keycloak`   | `quay.io/keycloak/keycloak:26.7.1`                          | `127.0.0.1:8443`  | Identity Provider OIDC SSO (`https://kc.localtest.me:8443/realms/patrolli`)     |
| `patrolli`   | `${PATROLLI_CORE_IMAGE}`                                    | `127.0.0.1:8080`  | Cluster utama Patrolli Core (local auth + OIDC)                                 |
| `patrolli_1` | `${PATROLLI_CORE_IMAGE}`                                    | `127.0.0.1:30001` | Cluster kedua untuk pengujian multi-cluster switching                           |
| `frontend`   | Playwright `webServer` (`npm run build && npm run preview`) | `3000`            | Server Nuxt/Nitro BFF (`https://localhost:3000`)                                |

---

## 2. Struktur Direktori `src/web/dev/`

```text
src/web/dev/
├── .dockerignore
├── docker-compose.yml               # Definisi stack backend E2E (memakai named volume postgres-data)
├── init.sql                         # Inisialisasi DB keycloak & patrolli1 saat Postgres pertama kali dibuat
├── setup.sh                         # Script utama: generate cert, secret ephemeral, dan render template
├── ssl.sh                           # Generator Root CA + server certs (frontend & keycloak)
├── certs/
│   ├── root.cnf                     # Konfigurasi Root CA
│   ├── frontend.cnf                 # SAN: localhost, 127.0.0.1
│   ├── keycloak.cnf                 # SAN: kc.localtest.me, localhost, 127.0.0.1
│   ├── csr/                         # (Gitignored) CSR sementara
│   └── ssl/                         # (Gitignored) *.crt, *.key, *.srl hasil generate
├── config/
│   ├── backend/
│   │   ├── patrolli-0-config.template.yaml
│   │   └── patrolli-1-config.template.yaml
│   ├── frontend/
│   │   └── config-fe.template.yaml
│   └── keycloak/
│       └── realm-config.template.json
├── .env                             # (Gitignored) Secret ephemeral untuk docker compose
└── .generated/                      # (Gitignored) Hasil render config runtime
```

---

## 3. Mekanisme Setup & Keamanan Secret

Tidak ada private key TLS maupun secret statis yang disimpan di Git:

1. **`./dev/ssl.sh`**:
    - Membuat CA, `frontend.crt/.key`, dan `keycloak.crt/.key` menggunakan `openssl`.
    - Menyetel `chmod 600` pada `ssl/ca.key` & `ssl/frontend.key`, serta `chmod 644` pada `ssl/*.crt` & `ssl/keycloak.key` agar container non-root (seperti Keycloak `uid 1000` di Linux runner) dapat membaca key yang di-mount `:ro`.
2. **`./dev/setup.sh`**:
    - Menjalankan `ssl.sh`.
    - Menghasilkan secret acak ephemeral (`E2E_JWT_SECRET`, `E2E_OIDC_SECRET`, `E2E_SECRET_KEY`, `E2E_SECRET_KEY_1`, `E2E_POSTGRES_PASSWORD`, `E2E_KC_BOOTSTRAP_PASSWORD`, `E2E_ADMIN_PASSWORD`) dan menyimpannya ke `dev/.env` (`chmod 600`). Semua nilai dapat di-override lewat environment variable sebelum menjalankan script.
    - Merender seluruh file `*.template.*` ke `dev/.generated/` (dengan `chmod 600` pada `config-fe.yaml`) dan menyalin `config-fe.yaml` ke `src/web/config-fe.yaml` (jika belum ada, atau selalu menimpa bila diberi flag `--force`).
    - Jika `GITHUB_ENV` aktif (di GitHub Actions), mem-mask password (`::add-mask::`) dan mengekspor `E2E_USERNAME=admin` serta `E2E_PASSWORD=${E2E_ADMIN_PASSWORD}`.
3. **`playwright.config.ts`**:
    - Secara lokal, jika `E2E_PASSWORD` belum diset di shell, `playwright.config.ts` hanya membaca `E2E_ADMIN_PASSWORD` dari `dev/.env` (memakai `node:util` `parseEnv`) tanpa memuat secret backend lainnya ke environment Playwright.
    - File frontend config dapat diarahkan lewat env `FRONTEND_CONFIG` (atau `CONFIG_PATH` / `CONFIG_FILE`), default fallback ke `src/web/config-fe.yaml`.

---

## 4. Menjalankan E2E di Device Baru (Lokal)

Prasyarat: **Docker & Docker Compose**, **Node.js 24**, **OpenSSL**.

```bash
cd src/web

# 1. Install dependencies & browser Playwright
npm ci
npx playwright install chromium --with-deps

# 2. Generate sertifikat TLS & config ephemeral
./dev/setup.sh --force

# 3. Nyalakan stack backend & tunggu healthy
docker compose -f dev/docker-compose.yml up -d --wait

# 4. Jalankan E2E tests
npm run test:e2e

# 5. Cleanup setelah selesai (opsional: -v menghapus volume Postgres)
docker compose -f dev/docker-compose.yml down -v
```

> [!NOTE]
> Jika offline atau DNS publik tidak me-resolve `*.localtest.me`, tambahkan `127.0.0.1 kc.localtest.me` ke `/etc/hosts`.

---

## 5. Panduan Integrasi GitHub Actions CI

### A. Strategi Image `patrolli-core` (Tanpa Compile Rust di PR Web)

Di `dev/docker-compose.yml`, image backend menggunakan:

```yaml
image: ${PATROLLI_CORE_IMAGE:-ghcr.io/bangunindo/patrolli-core:0.1.0-beta.3}
pull_policy: ${PATROLLI_CORE_PULL_POLICY:-always}
```

Rekomendasi alur untuk CI:

1. **Publish tag `:edge` saat `push` ke `main`** (jika `src/core/**` berubah):
    - Build image dari `src/core/Dockerfile` (`linux/amd64`) dan push ke `ghcr.io/bangunindo/patrolli-core:edge`.
    - Wajib pertahankan kepatuhan **AGPL-3.0** seperti di `.github/workflows/build.yaml`:
        - Label `org.opencontainers.image.licenses=AGPL-3.0-only` & `org.opencontainers.image.source=https://github.com/bangunindo/patrolli`.
        - `provenance: mode=max`, `sbom: true`, dan `actions/attest-build-provenance`.
        - Package GHCR harus **public read** agar PR dari fork eksternal tetap bisa `docker pull` tanpa secret.
2. **Saat `pull_request` berjalan**:
    - **Jika hanya `src/web/` berubah**: langsung pull `ghcr.io/bangunindo/patrolli-core:edge` (tanpa build Rust). `pull_policy: always` memastikan image tag terbaru selalu ditarik ulang.
    - **Jika `src/core/` ikut berubah**: build image lokal dari `src/core` (`docker build -t patrolli-core:pr-local src/core`) lalu set `PATROLLI_CORE_IMAGE=patrolli-core:pr-local` dan `PATROLLI_CORE_PULL_POLICY=never`.

### B. Referensi Job `web-e2e` untuk `.github/workflows/test.yaml`

Sesuai standar keamanan repo ini, **pin semua GitHub Actions ke commit SHA** dan set `persist-credentials: false`:

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

## 6. Kepatuhan Lisensi, Lint, & Isolasi Docker Build

- **AGPL-3.0 / REUSE (`fsfe/reuse-action`)**:
    - Setiap file baru di `src/web/dev/` wajib memiliki header `SPDX-FileCopyrightText` & `SPDX-License-Identifier` (`AGPL-3.0-only`), atau tercakup di `/REUSE.toml` (seperti `**/*.json`, `**/*.md`, `src/web/dev/certs/*.cnf`, dan `src/web/dev/.dockerignore`).
- **Prettier & ESLint (`npm run lint`)**:
    - `src/web/.prettierignore` mengabaikan `dev` agar file konfigurasi Compose/YAML/JSON tidak memicu error formatting saat `prettier --check .`.
- **Docker Build Hygiene (`src/web/.dockerignore`)**:
    - `.env`, `.env.*`, `config-fe.yaml`, `config-fe.yml`, dan `dev` sudah masuk `src/web/.dockerignore` sehingga artefak E2E lokal tidak ikut terbawa ke build image production `src/web/Dockerfile`.

---

## 7. Troubleshooting

- **Keycloak gagal membaca `/opt/keycloak/conf/keycloak.key` (`Permission denied`)**:
    - Jalankan ulang `./dev/ssl.sh` atau pastikan permission key `644`.
- **Setup Keycloak SSO timeout di halaman _"Update Account Information"_**:
    - User di `realm-config.template.json` wajib memiliki `email`, `firstName`, `lastName`, dan `"requiredActions": []`. Jika mengubah template setelah DB terlanjur terinisialisasi, reset volume dulu dengan `docker compose -f dev/docker-compose.yml down -v`.
- **Setup login gagal `401 Invalid credentials`**:
    - Pastikan container dijalankan setelah `./dev/setup.sh` agar `E2E_ADMIN_PASSWORD` di `dev/.env` sinkron dengan yang dibaca `playwright.config.ts`.
- **Pipeline test gagal dengan _"At least one host must be assigned"_**:
    - Pastikan `pipeline-builder.spec.ts` dan `pipeline-management.spec.ts` menggunakan helper `apiHelper.createHost(...)` (sudah diimplementasikan untuk berjalan bersih di DB kosong).
