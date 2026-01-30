# Build stage
# Note: ratatui-crossterm requires edition2024 which needs Rust 1.85+
FROM rust:1.85-bookworm AS builder

WORKDIR /app

# Install dependencies first for better caching
COPY Cargo.toml Cargo.lock ./

# Create dummy files to build dependencies (matching Cargo.toml structure)
RUN mkdir -p src benches && \
    echo "fn main() {}" > src/main.rs && \
    echo "fn main() {}" > benches/rendering.rs && \
    cargo build --release && \
    rm -rf src benches

# Copy actual source code
COPY src ./src
COPY benches ./benches
COPY tests ./tests

# Build the actual application (touch to update timestamps)
RUN touch src/main.rs && \
    cargo build --release --locked

# Runtime stage - using distroless for minimal attack surface
FROM gcr.io/distroless/cc-debian12 AS runtime

# Copy the binary from builder
COPY --from=builder /app/target/release/rct /usr/local/bin/rct

# Set working directory for file operations
WORKDIR /workspace

# Run as non-root user (distroless default is nonroot:65532)
USER nonroot:nonroot

# Set environment variables
ENV RUST_LOG=info

ENTRYPOINT ["/usr/local/bin/rct"]
