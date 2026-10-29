# Stonecrab interoperability tooling (slice S52b): tshark, the independent decoder of the
# Modbus/TCP packets that cross the Toxiproxy container. First-party recipe, Apache-2.0; the
# tool is Wireshark's (GPL-2.0-or-later), installed from Debian and never copied into this
# repository. Build with the directory of this file as the context:
#   docker build -t stonecrab-interop-tshark:local interop/toxiproxy/tshark

# debian:trixie-slim, the multi-architecture index digest pulled on 2026-10-28 (the same
# base as interop/libmodbus/Dockerfile).
FROM debian:trixie-slim@sha256:f6b08b09a05725f9431502d3b5714f5c4576e82c1a84fa1debe0c24a7e0c3ec4

ENV DEBIAN_FRONTEND=noninteractive

# The package version is not pinned here: Debian replaces a security release of tshark in
# the archive, and an exact pin would stop building within weeks. The version that was
# installed is recorded with every run (interop.sh evidence writes it to image-digest.txt).
RUN apt-get update \
    && apt-get install -y --no-install-recommends tshark \
    && rm -rf /var/lib/apt/lists/*

ENTRYPOINT ["tshark"]
