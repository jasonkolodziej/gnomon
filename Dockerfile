# syntax=docker/dockerfile:1.7
#
# Gnomon images: controller, node agent and CLI, from one multi-stage build.
#
#   docker build --target ctl   -t gnomon-ctl   .
#   docker build --target agent -t gnomon-agent .
#   docker build --target cli   -t gnomon-cli   .
#   docker build --target dist  --output type=local,dest=target/docker/linux .   # just the Linux binaries
#
# By default the binaries are compiled in the `builder` stage. The release workflow builds them
# natively per architecture first and passes --build-arg BINS=prebuilt, which copies
# dist/linux-<TARGETARCH>/ instead (no Rust build under emulation).

ARG RUST_VERSION=1
ARG DEBIAN_RELEASE=bookworm
ARG BINS=compiled

# --- compile -------------------------------------------------------------------------------
FROM rust:${RUST_VERSION}-${DEBIAN_RELEASE} AS builder
RUN apt-get update -q \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-install-recommends protobuf-compiler \
 && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY proto proto
COPY crates crates
RUN --mount=type=cache,id=gnomon-cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=gnomon-cargo-target,target=/src/target \
    cargo build --release --locked -p gnomon-ctl -p gnomon-agent -p gnomon-cli -p gnomon-top \
 && mkdir -p /out \
 && cd target/release \
 && cp gnomon-ctl gnomon-agent gnbatch gnqueue gncancel gninfo gntop /out/

FROM scratch AS compiled
COPY --from=builder /out/ /

FROM scratch AS prebuilt
ARG TARGETARCH
COPY dist/linux-${TARGETARCH}/ /

FROM ${BINS} AS bins

# Bare binaries, for `--output type=local` (used by testkit/multinode.sh --build-linux).
FROM scratch AS dist
COPY --from=bins / /

# --- runtime -------------------------------------------------------------------------------
FROM debian:${DEBIAN_RELEASE}-slim AS runtime
RUN apt-get update -q \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-install-recommends \
      ca-certificates tini procps \
 && rm -rf /var/lib/apt/lists/* \
 && groupadd --system --gid 7631 gnomon \
 && useradd --system --uid 7631 --gid gnomon --home-dir /var/lib/gnomon --create-home gnomon
LABEL org.opencontainers.image.source="https://github.com/jasonkolodziej/gnomon" \
      org.opencontainers.image.licenses="MIT"

# Controller. State is journalled to /var/lib/gnomon; mount a volume there to survive restarts.
FROM runtime AS ctl
COPY --from=bins /gnomon-ctl /gninfo /usr/local/bin/
ENV GNOMON_STATE_DIR=/var/lib/gnomon \
    GNOMON_ADDR=http://127.0.0.1:7631
VOLUME /var/lib/gnomon
EXPOSE 7631 7700
USER gnomon
HEALTHCHECK --interval=10s --timeout=5s --start-period=10s CMD ["gninfo"]
ENTRYPOINT ["tini", "--", "gnomon-ctl"]
LABEL org.opencontainers.image.title="gnomon-ctl" \
      org.opencontainers.image.description="Gnomon controller daemon (gRPC on :7631, Raft on :7700)"

# Node agent. Jobs run inside this container as the `gnomon` user, unconfined unless a writable
# cgroup2 root is mounted (see docs/INSTALL.md); for production nodes prefer the binary + systemd.
FROM runtime AS agent
COPY --from=bins /gnomon-agent /usr/local/bin/
ENV GNOMON_WORKDIR=/var/lib/gnomon/work
RUN install -d -o gnomon -g gnomon /var/lib/gnomon/work
USER gnomon
WORKDIR /var/lib/gnomon/work
ENTRYPOINT ["tini", "--", "gnomon-agent"]
LABEL org.opencontainers.image.title="gnomon-agent" \
      org.opencontainers.image.description="Gnomon node agent"

# Slurm-style CLI shims and the gntop live view. Point GNOMON_ADDR at the controller.
FROM runtime AS cli
COPY --from=bins /gnbatch /gnqueue /gncancel /gninfo /gntop /usr/local/bin/
# gnbatch submits as $USER; override with -e USER=<name> to submit as someone else.
ENV USER=gnomon
USER gnomon
WORKDIR /var/lib/gnomon
CMD ["gninfo"]
LABEL org.opencontainers.image.title="gnomon-cli" \
      org.opencontainers.image.description="Gnomon CLI: gnbatch, gnqueue, gncancel, gninfo, gntop"
