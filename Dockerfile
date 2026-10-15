# ============================================================
# Stage 1: Build the octos binary
# ============================================================
FROM rust:1.88-alpine AS builder

RUN apk add --no-cache musl-dev git pkgconfig

WORKDIR /src

# Cache dependencies
COPY Cargo.toml Cargo.lock ./
COPY crates/octos/octos-core/Cargo.toml crates/octos/octos-core/Cargo.toml
COPY crates/octos/octos-llm/Cargo.toml crates/octos/octos-llm/Cargo.toml
COPY crates/octos/octos-memory/Cargo.toml crates/octos/octos-memory/Cargo.toml
COPY crates/octos/octos-agent/Cargo.toml crates/octos/octos-agent/Cargo.toml
COPY crates/octos/octos-bus/Cargo.toml crates/octos/octos-bus/Cargo.toml
COPY crates/octos/octos-cli/Cargo.toml crates/octos/octos-cli/Cargo.toml
COPY crates/octos/octos-pipeline/Cargo.toml crates/octos/octos-pipeline/Cargo.toml
COPY crates/octos/octos-plugin/Cargo.toml crates/octos/octos-plugin/Cargo.toml
COPY crates/octos/app-skills/news/Cargo.toml crates/octos/app-skills/news/Cargo.toml
COPY crates/octos/app-skills/deep-search/Cargo.toml crates/octos/app-skills/deep-search/Cargo.toml
COPY crates/octos/app-skills/deep-crawl/Cargo.toml crates/octos/app-skills/deep-crawl/Cargo.toml
COPY crates/octos/app-skills/send-email/Cargo.toml crates/octos/app-skills/send-email/Cargo.toml
COPY crates/octos/app-skills/account-manager/Cargo.toml crates/octos/app-skills/account-manager/Cargo.toml
COPY crates/octos/app-skills/time/Cargo.toml crates/octos/app-skills/time/Cargo.toml
COPY crates/octos/app-skills/weather/Cargo.toml crates/octos/app-skills/weather/Cargo.toml
COPY crates/octos/platform-skills/voice/Cargo.toml crates/octos/platform-skills/voice/Cargo.toml

# Create stub source files for dependency caching
# Library crates get lib.rs, binary crates get main.rs
RUN mkdir -p crates/octos/octos-core/src && echo "" > crates/octos/octos-core/src/lib.rs && \
    mkdir -p crates/octos/octos-llm/src && echo "" > crates/octos/octos-llm/src/lib.rs && \
    mkdir -p crates/octos/octos-memory/src && echo "" > crates/octos/octos-memory/src/lib.rs && \
    mkdir -p crates/octos/octos-agent/src && echo "" > crates/octos/octos-agent/src/lib.rs && \
    mkdir -p crates/octos/octos-bus/src && echo "" > crates/octos/octos-bus/src/lib.rs && \
    mkdir -p crates/octos/octos-cli/src && echo "fn main() {}" > crates/octos/octos-cli/src/main.rs && \
    mkdir -p crates/octos/octos-pipeline/src && echo "" > crates/octos/octos-pipeline/src/lib.rs && \
    mkdir -p crates/octos/octos-plugin/src && echo "" > crates/octos/octos-plugin/src/lib.rs && \
    mkdir -p crates/octos/app-skills/news/src && echo "fn main() {}" > crates/octos/app-skills/news/src/main.rs && \
    mkdir -p crates/octos/app-skills/deep-search/src && echo "fn main() {}" > crates/octos/app-skills/deep-search/src/main.rs && \
    mkdir -p crates/octos/app-skills/deep-crawl/src && echo "fn main() {}" > crates/octos/app-skills/deep-crawl/src/main.rs && \
    mkdir -p crates/octos/app-skills/send-email/src && echo "fn main() {}" > crates/octos/app-skills/send-email/src/main.rs && \
    mkdir -p crates/octos/app-skills/account-manager/src && echo "fn main() {}" > crates/octos/app-skills/account-manager/src/main.rs && \
    mkdir -p crates/octos/app-skills/time/src && echo "fn main() {}" > crates/octos/app-skills/time/src/main.rs && \
    mkdir -p crates/octos/app-skills/weather/src && echo "fn main() {}" > crates/octos/app-skills/weather/src/main.rs && \
    mkdir -p crates/octos/platform-skills/voice/src && echo "fn main() {}" > crates/octos/platform-skills/voice/src/main.rs

RUN cargo build --release --bin octos \
      -p octos-cli \
      --features api,telegram,discord,slack,whatsapp,feishu,email,audio_mp3 \
      2>/dev/null || true

# Copy full source and build
COPY . .
RUN find crates -name '*.rs' -exec touch {} + && \
    cargo build --release --bin octos \
      -p octos-cli \
      --features api,telegram,discord,slack,whatsapp,feishu,email,matrix,audio_mp3

# ============================================================
# Stage 2: Minimal runtime image
# ============================================================
FROM alpine:3.21

RUN apk add --no-cache ca-certificates tzdata \
    # Runtime deps for skills (pptx, mofa-pptx, browser)
    nodejs npm ffmpeg chromium \
    # LibreOffice + Poppler for office document conversion and visual QA
    libreoffice poppler-utils \
    # GCC for soffice sandbox shim (compiled on first use if needed)
    gcc musl-dev

# Install Node.js skill dependencies
RUN npm install -g pptxgenjs react-icons react react-dom sharp

# Copy binary
COPY --from=builder /src/target/release/octos /usr/local/bin/octos

# Copy builtin skills
COPY --from=builder /src/crates/octos/octos-agent/skills /opt/octos/skills

# Create workspace
RUN mkdir -p /root/.octos/skills && \
    cp -r /opt/octos/skills/* /root/.octos/skills/ 2>/dev/null || true

ENTRYPOINT ["octos"]
CMD ["gateway"]
