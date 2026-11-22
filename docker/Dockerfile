# The Raven toolchain: raven, rvpm, and the runtime library, plus the C
# toolchain raven links programs with and git for rvpm dependencies.
#
# Build a program in this image and copy the binary into a small runtime
# image; compiled Raven programs do not need Raven installed to run.
FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends gcc libc6-dev git ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY raven/raven raven/rvpm raven/libraven_runtime.a /usr/local/bin/

WORKDIR /app
CMD ["raven", "--help"]
