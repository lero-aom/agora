FROM rust:1-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --locked --release -p agora-server

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/agora-server /usr/local/bin/agora-server
ENV AGORA_BIND_ADDR=0.0.0.0:8080
EXPOSE 8080
CMD ["agora-server"]
