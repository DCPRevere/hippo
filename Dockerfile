FROM rust:1.85-slim-bookworm AS builder
WORKDIR /app

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Cache dependencies
COPY Cargo.toml .
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

# Build real source
COPY src ./src
RUN touch src/main.rs && cargo build --release

FROM debian:bookworm-slim

# Non-root user: production deployments must not run hippo as root. The
# uid/gid are stable so volume mounts can be ACL'd against them.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 1000 hippo \
    && useradd --system --uid 1000 --gid hippo \
        --create-home --home-dir /home/hippo --shell /sbin/nologin hippo

COPY --from=builder /app/target/release/hippo /usr/local/bin/hippo
RUN chmod 0755 /usr/local/bin/hippo

USER hippo:hippo
WORKDIR /home/hippo
EXPOSE 21693
CMD ["hippo"]
