FROM ubuntu:24.04 AS rust-builder

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
RUN rustup target add wasm32-wasip1 wasm32-wasip2 x86_64-unknown-linux-musl

WORKDIR /workspace

COPY . .

RUN cargo build -p guest-example --target wasm32-wasip2 --release
RUN cargo build -p guest-call-legacy --target wasm32-wasip1 --release
RUN cargo build -p legacy-mock --target x86_64-unknown-linux-musl --release
RUN cargo build -p tachyon-cli --release
RUN ./target/release/tachyon-cli generate --route /api/guest-example --route /api/guest-call-legacy --route /api/guest-go --route /api/guest-js --route /api/guest-csharp --route /api/guest-java --memory 64
RUN cargo build -p core-host --target x86_64-unknown-linux-musl --release

FROM ubuntu:24.04 AS tinygo-builder

ARG DEBIAN_FRONTEND=noninteractive
ARG TINYGO_VERSION=0.40.1

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    golang-go \
    && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL -o /tmp/tinygo.tar.gz https://github.com/tinygo-org/tinygo/releases/download/v${TINYGO_VERSION}/tinygo${TINYGO_VERSION}.linux-amd64.tar.gz \
    && tar -C /usr/local -xzf /tmp/tinygo.tar.gz \
    && ln -s /usr/local/tinygo/bin/tinygo /usr/local/bin/tinygo \
    && rm /tmp/tinygo.tar.gz

WORKDIR /workspace/guest-go

COPY guest-go/go.mod ./
COPY guest-go/main.go ./

RUN mkdir -p /workspace/guest-modules \
    && tinygo build -o /workspace/guest-modules/guest_go.wasm -target=wasip1 .

FROM ubuntu:24.04 AS javy-builder

ARG DEBIAN_FRONTEND=noninteractive
ARG JAVY_VERSION=8.1.0

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    gzip \
    && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL -o /tmp/javy.gz https://github.com/bytecodealliance/javy/releases/download/v${JAVY_VERSION}/javy-x86_64-linux-v${JAVY_VERSION}.gz \
    && gzip -d /tmp/javy.gz \
    && install -m 0755 /tmp/javy /usr/local/bin/javy \
    && rm /tmp/javy

WORKDIR /workspace/guest-js

COPY guest-js/index.js ./

RUN mkdir -p /workspace/guest-modules \
    && javy build /workspace/guest-js/index.js -o /workspace/guest-modules/guest_js.wasm

FROM mcr.microsoft.com/dotnet/sdk:8.0 AS dotnet-builder

ARG DEBIAN_FRONTEND=noninteractive
ARG WASI_SDK_VERSION=20.0

ENV DOTNET_CLI_TELEMETRY_OPTOUT=1
ENV WASI_SDK_PATH=/opt/wasi-sdk-${WASI_SDK_VERSION}

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    && rm -rf /var/lib/apt/lists/*

RUN dotnet workload install wasi-experimental
RUN curl -fsSL -o /tmp/wasi-sdk.tar.gz https://github.com/WebAssembly/wasi-sdk/releases/download/wasi-sdk-20/wasi-sdk-${WASI_SDK_VERSION}-linux.tar.gz \
    && tar -C /opt -xzf /tmp/wasi-sdk.tar.gz \
    && rm /tmp/wasi-sdk.tar.gz

WORKDIR /workspace/guest-csharp

COPY guest-csharp/guest-csharp.csproj ./
COPY guest-csharp/Program.cs ./

RUN mkdir -p /workspace/guest-modules \
    && dotnet publish guest-csharp.csproj -c Release \
    && cp /workspace/guest-csharp/bin/Release/net8.0/wasi-wasm/AppBundle/guest_csharp.wasm /workspace/guest-modules/guest_csharp.wasm

FROM maven:3.9.11-eclipse-temurin-17 AS java-builder

WORKDIR /workspace/guest-java

COPY guest-java/pom.xml ./
COPY guest-java/src ./src

RUN mkdir -p /workspace/guest-modules \
    && mvn -B --no-transfer-progress -e clean package \
    && cp /workspace/guest-java/target/teavm-wasi/guest_java.wasm /workspace/guest-modules/guest_java.wasm

FROM scratch AS legacy-runtime

WORKDIR /app

COPY --from=rust-builder /workspace/target/x86_64-unknown-linux-musl/release/legacy-mock /app/legacy-mock

EXPOSE 8081

ENTRYPOINT ["/app/legacy-mock"]

FROM scratch AS runtime

WORKDIR /app

COPY --from=rust-builder /workspace/target/x86_64-unknown-linux-musl/release/core-host /app/core-host
COPY --from=rust-builder /workspace/target/wasm32-wasip2/release/guest_example.wasm /app/guest-modules/guest_example.wasm
COPY --from=rust-builder /workspace/target/wasm32-wasip1/release/guest_call_legacy.wasm /app/guest-modules/guest_call_legacy.wasm
COPY --from=tinygo-builder /workspace/guest-modules/guest_go.wasm /app/guest-modules/guest_go.wasm
COPY --from=javy-builder /workspace/guest-modules/guest_js.wasm /app/guest-modules/guest_js.wasm
COPY --from=dotnet-builder /workspace/guest-modules/guest_csharp.wasm /app/guest-modules/guest_csharp.wasm
COPY --from=java-builder /workspace/guest-modules/guest_java.wasm /app/guest-modules/guest_java.wasm

EXPOSE 8080

ENTRYPOINT ["/app/core-host"]
