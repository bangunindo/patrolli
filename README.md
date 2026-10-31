# Patrolli

Patrolli is a monitoring and telemetry platform. It collects data from network
devices and services, processes it, and delivers it to the storage and
analytics systems you already use.

Many networks still depend on SNMP, syslog, and NetFlow. Patrolli receives that
data, converts it to OpenTelemetry, and keeps it in the
[OpenTelemetry Arrow](https://github.com/open-telemetry/otel-arrow) format from
end to end.

- **Collect:** listen for syslog, NetFlow/sFlow, SNMP traps, and OTLP; poll
  with SNMP get/walk and Prometheus scrape.
- **Process:** filter, rename, enrich from lookup tables, or run a transform
  program.
- **Deliver:** to files, Kafka, ClickHouse, or any OpenTelemetry endpoint.

It runs as a single node from one YAML file, or as a cluster backed by
PostgreSQL and managed through the REST API and web interface.

## Repository layout

| Path | Contents |
| --- | --- |
| `src/core` | The Patrolli server (Rust) |
| `src/web` | The web interface (Nuxt) |
| `src/docs` | Documentation site (VitePress) |

## Documentation

Start with [What is Patrolli?](src/docs/en/introduction/what-is-patrolli.md),
then the [deployment guide](src/docs/en/getting-started/deployment/).

## Contributing

We don't accept pull requests yet; bug reports and feature requests are
welcome as issues. See [CONTRIBUTING.md](CONTRIBUTING.md), and report security
problems as described in [SECURITY.md](SECURITY.md).

## License

Copyright © 2024-2026 PT Bangunindo Teknusa Jaya.

Patrolli is free software: you can redistribute it and/or modify it under the
terms of the [GNU Affero General Public License, version 3](LICENSE)
(`AGPL-3.0-only`).

If you run a modified version of Patrolli as a network service, the AGPL
requires you to offer its source code to the users of that service. The web
interface links to the source from the user menu and the sign-in page; point
`sourceUrl` in `src/web/shared/constants/app.constant.ts` at your fork.

Every file carries an SPDX license header or is covered by
[`REUSE.toml`](REUSE.toml), following the [REUSE](https://reuse.software)
specification; license texts are in [`LICENSES/`](LICENSES). Bundled
third-party code is listed in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
