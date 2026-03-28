FROM ubuntu:24.04 AS builder

ARG DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    curl \
    file \
    libayatana-appindicator3-dev \
    librsvg2-dev \
    libssl-dev \
    libwebkit2gtk-4.1-dev \
    libxdo-dev \
    musl-tools \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

ENV CARGO_HOME=/usr/local/cargo
ENV PATH="${CARGO_HOME}/bin:${PATH}"
ENV RUSTUP_HOME=/usr/local/rustup

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
RUN rustup target add wasm32-wasip1 x86_64-unknown-linux-musl

WORKDIR /workspace

COPY . .

RUN cargo build -p guest-example --target wasm32-wasip1 --release
RUN cargo build -p guest-call-legacy --target wasm32-wasip1 --release
RUN cargo build -p legacy-mock --target x86_64-unknown-linux-musl --release
RUN cargo build -p tachyon-cli --release
RUN ./target/release/tachyon-cli generate --route /api/guest-example --route /api/guest-call-legacy --memory 64
RUN cargo build -p core-host --target x86_64-unknown-linux-musl --release

FROM scratch AS legacy-runtime

WORKDIR /app

COPY --from=builder /workspace/target/x86_64-unknown-linux-musl/release/legacy-mock /app/legacy-mock

EXPOSE 8081

ENTRYPOINT ["/app/legacy-mock"]

FROM scratch AS runtime

WORKDIR /app

COPY --from=builder /workspace/target/x86_64-unknown-linux-musl/release/core-host /app/core-host
COPY --from=builder /workspace/target/wasm32-wasip1/release/guest_example.wasm /app/guest-modules/guest_example.wasm
COPY --from=builder /workspace/target/wasm32-wasip1/release/guest_call_legacy.wasm /app/guest-modules/guest_call_legacy.wasm

EXPOSE 8080

ENTRYPOINT ["/app/core-host"]
