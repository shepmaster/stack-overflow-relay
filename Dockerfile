# syntax=docker/dockerfile:1
FROM rust:latest AS builder

RUN cargo install diesel_cli --no-default-features --features "postgres" --version "2.0.0-rc.1"

ARG SOURCE_VERSION

# Pre-build the dependencies
RUN cargo new app
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN sed -i 's/^alictor/#alictor/' Cargo.toml
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release

# Build the code
COPY . /app
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/app/target \
    cargo build --release && \
    cp /app/target/release/stack-overflow-relay /app/

# ----------

FROM debian:bullseye-slim

ENV DEBIAN_FRONTEND=noninteractive
RUN apt-get update && \
    apt-get install -y \
    ca-certificates \
    libpq5 \
    && \
    apt-get clean

RUN mkdir /app
RUN useradd -ms /bin/bash app
USER app

COPY --from=builder /usr/local/cargo/bin/diesel /app/
COPY --from=builder /app/stack-overflow-relay /app/
COPY --from=builder /app/migrations /app/migrations
WORKDIR /app

CMD /app/stack-overflow-relay
