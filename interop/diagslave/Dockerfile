# Stonecrab interoperability peer (slice S61): diagslave 3.6, the free Modbus slave simulator of
# proconX (https://www.modbusdriver.com/diagslave.html), serving Modbus UDP ("-m udp") or, since
# slice S59b, Modbus RTU over TCP ("-m enc"; serve.sh takes the mode), with iproute2 for the
# "tc netem" faults the UDP tests apply inside the container. First-party recipe,
# Apache-2.0; diagslave itself is not in this repository and the image is never published: the
# build downloads the archive and checks it by SHA-256 (see PROVENANCE.toml for its license).
# Build with the repository root as the context:
#   docker build -f interop/diagslave/Dockerfile -t stonecrab-interop-diagslave:local .
# (interop/diagslave/Dockerfile.dockerignore keeps the context to the file copied below.)

# debian:trixie-slim, the multi-architecture index digest pulled on 2026-10-28 (Debian 13.7),
# the base of interop/libmodbus/ as well.
FROM debian:trixie-slim@sha256:f6b08b09a05725f9431502d3b5714f5c4576e82c1a84fa1debe0c24a7e0c3ec4

ENV DEBIAN_FRONTEND=noninteractive

# tc for netem. The package version is pinned: a trixie that no longer has it fails this step.
RUN apt-get update \
    && apt-get install -y --no-install-recommends iproute2=6.15.0-1 \
    && rm -rf /var/lib/apt/lists/*

# The download URL has no version in it, so the vendor can replace the archive under the same
# name. The checksum is that of diagslave 3.6 (archive member diagslave-3.6.tgz, dated
# 2026-10-10), fetched on 2026-10-29: another archive fails this step with a digest mismatch
# instead of building another simulator.
ADD --checksum=sha256:01c87fe49d86848d61ddfdd703c0f9f76f5e80e46f233d5ee16998811f2fe97e \
    https://www.modbusdriver.com/downloads/diagslave.tgz /tmp/diagslave.tgz

# The archive holds statically linked binaries for four Linux targets; take the one of this
# image, keep the license and the read-me beside it, and drop the rest.
ARG TARGETARCH
RUN case "${TARGETARCH:-$(dpkg --print-architecture)}" in \
         amd64) target=x86_64-linux-gnu ;; \
         arm64) target=aarch64-linux-gnu ;; \
         *) echo "diagslave 3.6 has no binary for ${TARGETARCH}" >&2; exit 1 ;; \
       esac \
    && mkdir -p /opt/diagslave \
    && tar -xzf /tmp/diagslave.tgz -C /tmp --no-same-owner \
    && install -m 0755 "/tmp/diagslave/${target}/diagslave" /usr/local/bin/diagslave \
    && cp /tmp/diagslave/LICENSE-FREE.txt /tmp/diagslave/README.txt /opt/diagslave/ \
    && rm -rf /tmp/diagslave /tmp/diagslave.tgz

COPY interop/diagslave/serve.sh /usr/local/bin/serve.sh

# diagslave needs no privileges: port 1502 is above 1023. The tests run tc as root through
# docker exec, which the container's NET_ADMIN capability allows.
RUN useradd --system --no-create-home --uid 10001 modbus
USER 10001

EXPOSE 1502/udp 1503/tcp
ENTRYPOINT ["/usr/local/bin/serve.sh"]
