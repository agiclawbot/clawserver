# =============================================================================
# Stage 1: Builder
# =============================================================================
FROM rust:1.79-bookworm AS builder

WORKDIR /app

# 先复制依赖描述文件，利用 Docker 缓存层避免每次重编所有依赖
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/

# 创建一个虚拟 main.rs 让 cargo 先编译依赖（缓存层）
RUN mkdir src && echo "fn main() {}" > src/main.rs && \
    cargo build --release 2>/dev/null || true && \
    rm -rf src && cargo clean -p clawserver 2>/dev/null; true

# 真正复制源码
COPY src/ ./src/
COPY examples/ ./examples/

# 生产构建（profile.release 已设 LTO + codegen-units=1 + panic=abort）
RUN cargo build --release --bin clawserver && \
    cargo build --release --bin clawctl && \
    cp target/release/clawserver /tmp/clawserver && \
    cp target/release/clawctl /tmp/clawctl && \
    strip /tmp/clawserver /tmp/clawctl

# =============================================================================
# Stage 2: Runtime
# =============================================================================
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# 时区、非 root 用户
RUN groupadd -r claw && useradd -r -g claw -d /app -s /sbin/nologin claw

WORKDIR /app

COPY --from=builder /tmp/clawserver /usr/local/bin/clawserver
COPY --from=builder /tmp/clawctl   /usr/local/bin/clawctl

# 默认配置（用户可通过 volume mount 覆盖）
COPY config/ ./config/

EXPOSE 3385

USER claw

HEALTHCHECK --interval=15s --timeout=3s --start-period=5s --retries=2 \
    CMD ["clawctl", "health"]

ENV CLAW_CONFIG_DIR=/app/config

CMD ["clawserver"]
