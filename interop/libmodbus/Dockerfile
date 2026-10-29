# Stonecrab interoperability peer (slice S52a): libmodbus 3.1.11's unit-test-server, the C
# stack of the libmodbus test suite, as Debian trixie packages it. First-party recipe,
# Apache-2.0; the server's source is not in this repository: the build compiles the copy
# that Debian's libmodbus-dev ships. Build with the repository root as the context:
#   docker build -f interop/libmodbus/Dockerfile -t stonecrab-interop-libmodbus:local .
# (interop/libmodbus/Dockerfile.dockerignore keeps the context to the file copied below.)

# debian:trixie-slim, the multi-architecture index digest pulled on 2026-10-28.
FROM debian:trixie-slim@sha256:f6b08b09a05725f9431502d3b5714f5c4576e82c1a84fa1debe0c24a7e0c3ec4 AS build

ENV DEBIAN_FRONTEND=noninteractive

# The slim image drops /usr/share/doc, which is where libmodbus-dev puts the test programs,
# so lift that rule before the install. The package version is pinned: a trixie that no
# longer has it fails this step, it does not build another server.
RUN rm -f /etc/dpkg/dpkg.cfg.d/docker \
    && apt-get update \
    && apt-get install -y --no-install-recommends gcc libc6-dev \
         libmodbus5=3.1.11-2 libmodbus-dev=3.1.11-2 \
    && rm -rf /var/lib/apt/lists/*

# The two files of the package that make the server, by checksum, so that what is compiled is
# what was reviewed (both are the upstream tests/ files of 3.1.11, BSD-3-Clause).
RUN cd /usr/share/doc/libmodbus-dev/examples \
    && printf '%s  %s\n' \
         62d3dc09d059410413a9ecdcc5efa1118cc482941c12a6d2dae13684513b86c3 unit-test-server.c \
         84a20de4d89b3cb2c30270fc82aecd1e2b22afedfd17b2a2f463b456a5e597bf unit-test.h \
       | sha256sum -c - \
    && mkdir /out \
    && gcc -O2 -Wall -I/usr/include/modbus -o /out/unit-test-server unit-test-server.c -lmodbus

FROM debian:trixie-slim@sha256:f6b08b09a05725f9431502d3b5714f5c4576e82c1a84fa1debe0c24a7e0c3ec4

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends libmodbus5=3.1.11-2 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /out/unit-test-server /usr/local/bin/unit-test-server
COPY interop/libmodbus/serve.sh /usr/local/bin/serve.sh

# The server needs no privileges: port 1502 is above 1023 and it writes nothing.
RUN useradd --system --no-create-home --uid 10001 modbus
USER 10001

EXPOSE 1502
ENTRYPOINT ["/usr/local/bin/serve.sh"]
