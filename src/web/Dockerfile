# SPDX-FileCopyrightText: 2026 PT Bangunindo Teknusa Jaya
#
# SPDX-License-Identifier: AGPL-3.0-only

FROM node:24-slim AS build-stage

WORKDIR /app

COPY . .

RUN npm ci

RUN npm run build

FROM node:24-alpine AS production-stage
LABEL org.opencontainers.image.source="https://github.com/bangunindo/patrolli" \
      org.opencontainers.image.licenses="AGPL-3.0-only"

WORKDIR /app

ENV NODE_ENV=production
ENV PORT=3000
ENV HOST=0.0.0.0

COPY --from=build-stage /app/.output ./.output
COPY LICENSE ./LICENSE

EXPOSE 3000

CMD ["node", ".output/server/index.mjs"]
