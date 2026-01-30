# Build stage
# Note: ratatui 0.30+ requires Rust 1.86+, time 0.3.46+ requires Rust 1.88+
# Using Rust 1.93 (current stable LTS)
FROM rust:1.93-bookworm AS builder

WORKDIR /app

# Install dependencies first for better caching
COPY Cargo.toml Cargo.lock ./

# Create dummy files to build dependencies (matching Cargo.toml structure)
# Includes: main binary, benchmark, and test binary (mock_mcp_server)
RUN mkdir -p src benches tests/helpers && \
    echo "fn main() {}" > src/main.rs && \
    echo "fn main() {}" > benches/rendering.rs && \
    echo "fn main() {}" > tests/helpers/mock_mcp_server.rs && \
    cargo build --release && \
    rm -rf src benches tests

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
