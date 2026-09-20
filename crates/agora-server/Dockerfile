# syntax=docker/dockerfile:1.7@sha256:a57df69d0ea827fb7266491f2813635de6f17269be881f696fbfdf2d83dda33e

# Keep the default build inputs immutable. Maintainers can replace either
# argument with a reviewed tag@sha256 reference when refreshing base images.
ARG RUST_IMAGE=rust:1-bookworm@sha256:828077e0f5ed0401fbd9cb5b4d5dedca23bd13c7fe032f3e8b7313e7acd2a57f
ARG RUNTIME_IMAGE=debian:bookworm-slim@sha256:3783cc01769c7b2b1b83a5c5ad96c815348e28ed7da68e2e3687004faa906251

FROM ${RUST_IMAGE} AS builder
WORKDIR /app
COPY . .
RUN cargo build --locked --release -p agora-server

FROM ${RUNTIME_IMAGE}
ARG VERSION=unknown
ARG REVISION=unknown
LABEL org.opencontainers.image.title="Agora server" \
      org.opencontainers.image.description="Agora chat server" \
      org.opencontainers.image.licenses="AGPL-3.0-only" \
      org.opencontainers.image.source="https://github.com/lero-aom/agora" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}"

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates wget \
    && groupadd --gid 10001 agora \
    && useradd --uid 10001 --gid agora --create-home --home-dir /app --shell /usr/sbin/nologin agora \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder --chown=10001:10001 /app/target/release/agora-server /usr/local/bin/agora-server
ENV AGORA_BIND_ADDR=0.0.0.0:8080
ENV HOME=/app
WORKDIR /app
USER 10001:10001
EXPOSE 8080
HEALTHCHECK --interval=10s --timeout=5s --start-period=60s --retries=12 CMD ["wget", "--quiet", "--spider", "http://127.0.0.1:8080/health"]
CMD ["agora-server"]
