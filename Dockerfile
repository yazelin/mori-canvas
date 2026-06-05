# syntax=docker/dockerfile:1
# Mori Canvas — self-contained image. The Rust binary embeds the built client
# (include_dir) and serves client + /api + /sync on one port. For Render etc.

# 1) build the React/Konva client -> client/dist
FROM node:20-slim AS client
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci
COPY vite.config.ts tsconfig.json ./
COPY client ./client
RUN npm run build:client

# 2) build the Rust server (embeds client/dist at compile time; rustls => no OpenSSL)
FROM rust:1-bookworm AS server
WORKDIR /app
COPY server-rs ./server-rs
COPY --from=client /app/client/dist ./client/dist
COPY prompts ./prompts
RUN cargo build --release --manifest-path server-rs/Cargo.toml

# 3) runtime — ffmpeg for STT silence-trim, ca-certificates for HTTPS to Groq
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates ffmpeg \
    && rm -rf /var/lib/apt/lists/*
COPY --from=server /app/server-rs/target/release/mori-canvas-server /usr/local/bin/mori-canvas-server
# Render/most PaaS set PORT; bind all interfaces. TLS is terminated at the platform edge,
# so the container serves plain HTTP (do NOT set HTTPS=1 here).
ENV BIND=0.0.0.0 PORT=1334
EXPOSE 1334
CMD ["mori-canvas-server"]
