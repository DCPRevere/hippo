FROM rust:1.94-slim-bookworm AS builder
WORKDIR /app

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Cache dependencies. The workspace has three members (root binary,
# hippo-api, hippo-wasm); each one's Cargo.toml must be present before
# `cargo build` will resolve the workspace graph, and each one needs a
# stub source root so the compile step has something to look at. Only
# the default members (".", "hippo-api") are actually compiled — but
# `hippo-wasm`'s Cargo.toml still has to be readable for the workspace
# to resolve at all.
COPY Cargo.toml Cargo.lock ./
COPY hippo-api/Cargo.toml ./hippo-api/Cargo.toml
COPY hippo-wasm/Cargo.toml ./hippo-wasm/Cargo.toml
RUN mkdir -p src/bin benches hippo-api/src hippo-wasm/src \
    && echo "fn main() {}" > src/main.rs \
    && echo "fn main() {}" > src/bin/mcp_server.rs \
    && echo "fn main() {}" > src/bin/eval_regression.rs \
    && echo "fn main() {}" > src/bin/eval_score.rs \
    && echo "fn main() {}" > src/bin/cli.rs \
    && echo "fn main() {}" > benches/benchmark.rs \
    && echo "" > hippo-api/src/lib.rs \
    && echo "" > hippo-wasm/src/lib.rs
RUN cargo build --release --bin hippo
RUN rm -rf src benches hippo-api/src hippo-wasm/src

# Build real source. We only need the source for the default workspace
# members; hippo-wasm is built separately via wasm-pack, not as part of
# the server image. We do still need a lib.rs stub for hippo-wasm so
# cargo can resolve the workspace.
COPY src ./src
COPY benches ./benches
COPY hippo-api/src ./hippo-api/src
# docs/openapi.yaml is `include_str!`'d at build time by src/openapi.rs.
COPY docs ./docs
RUN mkdir -p hippo-wasm/src && echo "" > hippo-wasm/src/lib.rs
RUN touch src/main.rs hippo-api/src/lib.rs && cargo build --release --bin hippo

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
