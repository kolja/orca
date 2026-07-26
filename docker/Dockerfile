FROM --platform=$BUILDPLATFORM alpine:3.22 AS fetch

ARG VERSION
ARG TARGETARCH

RUN apk add --no-cache curl tar

WORKDIR /out

# see: .github/workflows/rust.yml
RUN case "$TARGETARCH" in \
      amd64) RUST_TARGET=x86_64-unknown-linux-musl ;; \
      arm64) RUST_TARGET=aarch64-unknown-linux-musl ;; \
      *) echo "unsupported TARGETARCH: ${TARGETARCH}" >&2; exit 1 ;; \
    esac && \
    curl -fsSL -o orca.tar.gz \
      "https://github.com/kolja/orca/releases/download/${VERSION}/orca-${RUST_TARGET}.tar.gz" && \
    tar -xzf orca.tar.gz && \
    chmod 755 orca && \
    rm orca.tar.gz

FROM scratch

COPY --from=fetch /out/orca /app/orca

WORKDIR /app

USER 65532:65532

# scratch has no /etc/passwd and no $HOME, so config.rs's fallback to
# ~/.config/orca.toml cannot resolve: ORCA_CONFIG must be set and the config
# file mounted into the container.
CMD ["/app/orca"]
