FROM ubuntu:24.04 AS builder

ARG DEBIAN_FRONTEND=noninteractive
ARG TINYGO_VERSION=0.40.1
ARG JAVY_VERSION=8.1.0

RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential \
    ca-certificates \
    curl \
    file \
    golang-go \
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
RUN curl -fsSL -o /tmp/tinygo.tar.gz https://github.com/tinygo-org/tinygo/releases/download/v${TINYGO_VERSION}/tinygo${TINYGO_VERSION}.linux-amd64.tar.gz \
    && tar -C /usr/local -xzf /tmp/tinygo.tar.gz \
    && ln -s /usr/local/tinygo/bin/tinygo /usr/local/bin/tinygo \
    && rm /tmp/tinygo.tar.gz
RUN curl -fsSL -o /tmp/javy.gz https://github.com/bytecodealliance/javy/releases/download/v${JAVY_VERSION}/javy-x86_64-linux-v${JAVY_VERSION}.gz \
    && gzip -d /tmp/javy.gz \
    && install -m 0755 /tmp/javy /usr/local/bin/javy \
    && rm /tmp/javy

WORKDIR /workspace

COPY . .

RUN mkdir -p guest-modules
RUN cargo build -p guest-example --target wasm32-wasip1 --release
RUN cargo build -p guest-call-legacy --target wasm32-wasip1 --release
RUN cd /workspace/guest-go && tinygo build -o /workspace/guest-modules/guest_go.wasm -target=wasip1 .
RUN javy build /workspace/guest-js/index.js -o /workspace/guest-modules/guest_js.wasm
RUN cargo build -p legacy-mock --target x86_64-unknown-linux-musl --release
RUN cargo build -p tachyon-cli --release
RUN ./target/release/tachyon-cli generate --route /api/guest-example --route /api/guest-call-legacy --route /api/guest-go --route /api/guest-js --memory 64
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
COPY --from=builder /workspace/guest-modules/guest_go.wasm /app/guest-modules/guest_go.wasm
COPY --from=builder /workspace/guest-modules/guest_js.wasm /app/guest-modules/guest_js.wasm

EXPOSE 8080

ENTRYPOINT ["/app/core-host"]
