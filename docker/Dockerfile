# CLI image — musl static binary on distroless.

FROM rust:1.90-slim AS builder
RUN apt-get update && apt-get install -y --no-install-recommends \
    musl-tools pkg-config \
    && rm -rf /var/lib/apt/lists/*
RUN rustup target add x86_64-unknown-linux-musl
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
RUN cargo build --release --locked -p mcp-gateway-cli --target x86_64-unknown-linux-musl

FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=builder /src/target/x86_64-unknown-linux-musl/release/mcp-gateway /usr/local/bin/mcp-gateway
USER nonroot
EXPOSE 8787
ENTRYPOINT ["/usr/local/bin/mcp-gateway"]
CMD ["--help"]
