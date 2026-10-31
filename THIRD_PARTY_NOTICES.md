# Third-party notices

Patrolli is licensed under `AGPL-3.0-only`. This repository also contains the
third-party material below, which keeps its original license. The full
license texts are in [`LICENSES/`](LICENSES), and the per-file details are in
[`REUSE.toml`](REUSE.toml).

## shadcn-vue

- **Files:** `src/web/app/components/ui/`
- **Source:** https://github.com/unovue/shadcn-vue, a port of
  https://github.com/shadcn-ui/ui
- **License:** MIT ([`LICENSES/MIT.txt`](LICENSES/MIT.txt))
- **Copyright:** © 2023 unovue; © 2023 shadcn

UI components copied into this repository and adapted for Patrolli. Local
changes are also released under the MIT license.

## IETF and IANA MIB modules

- **Files:** `src/core/tests/mibs/` (test fixtures only)
- **Source:** SNMPv2-MIB (RFC 3418), IF-MIB (RFC 2863), IANAifType-MIB
  (maintained by IANA)
- **License:** the RFC copyright statement
  ([`LICENSES/LicenseRef-ISOC-RFC.txt`](LICENSES/LicenseRef-ISOC-RFC.txt))
- **Copyright:** © The Internet Society

## Dependencies

Rust crates and npm packages are fetched at build time and are not
redistributed in this repository. Each is under its own license; the allowed
set for Rust is enforced by `cargo deny check licenses` (see
`src/core/deny.toml`). Published container images include an SBOM listing
every component they contain.
