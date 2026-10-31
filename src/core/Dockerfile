# SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
#
# SPDX-License-Identifier: AGPL-3.0-only

FROM rust:1.99.1 AS build_stage
RUN apt-get update && apt-get install -y \
    cmake \
    libclang-dev \
    libcurl4-openssl-dev \
    libsasl2-dev \
    protobuf-compiler \
    libunwind-dev
COPY . /source_code/
WORKDIR /source_code
RUN cargo build --release --bin patrolli
RUN chmod +x ./target/release/patrolli

FROM ubuntu:26.04
LABEL org.opencontainers.image.source="https://github.com/bangunindo/patrolli" \
      org.opencontainers.image.licenses="AGPL-3.0-only"
ENV DEBIAN_FRONTEND=noninteractive TZ=Etc/UTC
RUN apt-get update && \
    apt-get install -y \
    libsasl2-dev \
    snmp-mibs-downloader \
    ca-certificates \
    libunwind8
COPY --from=build_stage /source_code/target/release/patrolli /patrolli
COPY LICENSE /LICENSE
ENTRYPOINT ["/patrolli"]
CMD ["--config", "config.yaml"]