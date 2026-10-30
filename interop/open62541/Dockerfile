# Stonecrab interoperability peer (slice S81): an OPC UA server and a check client on
# open62541 v1.5.9, built from the source at the pinned tag commit with its OpenSSL
# encryption backend. First-party recipe, Apache-2.0; open62541 (MPL-2.0) is fetched and built
# inside the image and none of its source is in this repository. Build with the repository
# root as the context:
#   docker build -f interop/open62541/Dockerfile -t stonecrab-interop-open62541:local .
# (interop/open62541/Dockerfile.dockerignore keeps the context to the files copied below.)

# debian:trixie-slim, the multi-architecture index digest pulled on 2026-10-28 (the base of
# interop/libmodbus/Dockerfile).
FROM debian:trixie-slim@sha256:f6b08b09a05725f9431502d3b5714f5c4576e82c1a84fa1debe0c24a7e0c3ec4 AS build

ENV DEBIAN_FRONTEND=noninteractive

# The build tools are not pinned: they are not in the final image. OpenSSL (libssl-dev here,
# libssl3t64 and openssl below) is the archive's current 3.5 of Debian 13; the security archive
# keeps only the newest version of a package, so an exact pin would stop the weekly scheduled
# run at the first security update. The versions of a build are recorded in the evidence.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git cmake make gcc libc6-dev \
         libssl-dev python3 \
    && rm -rf /var/lib/apt/lists/*

# v1.5.9 is a lightweight tag; this is the commit it names (resolved with git ls-remote on
# 2026-10-30). The fetch is by tag and the checkout is refused when the tag names any other
# commit, so a moved tag cannot change what is built.
ENV OPEN62541_TAG=v1.5.9 \
    OPEN62541_COMMIT=05c2b389b65cc35884eb40d800301af1f3d130cf
RUN git init -q /src \
    && cd /src \
    && git fetch -q --depth 1 https://github.com/open62541/open62541.git "refs/tags/${OPEN62541_TAG}:refs/tags/${OPEN62541_TAG}" \
    && git checkout -q --detach "${OPEN62541_TAG}" \
    && test "$(git rev-parse HEAD)" = "${OPEN62541_COMMIT}" \
    && grep -q 'OPEN62541_VER_PATCH 9' CMakeLists.txt

# Static library, encryption with OpenSSL (not mbedTLS), the reduced namespace zero (it holds
# BaseEventType), subscriptions with events. Nothing else: no examples, tools or tests. Link
# time optimization is off: open62541 turns it on in a release build, and gcc's lto1 crashed
# linking the static library under amd64 emulation on an arm64 host.
RUN cmake -S /src -B /build -DCMAKE_BUILD_TYPE=Release -DBUILD_SHARED_LIBS=OFF \
      -DCMAKE_INTERPROCEDURAL_OPTIMIZATION=OFF \
      -DUA_ENABLE_ENCRYPTION=OPENSSL -DUA_NAMESPACE_ZERO=REDUCED \
      -DUA_ENABLE_SUBSCRIPTIONS_EVENTS=ON \
      -DUA_BUILD_EXAMPLES=OFF -DUA_BUILD_TOOLS=OFF -DUA_BUILD_UNIT_TESTS=OFF \
      -DCMAKE_INSTALL_PREFIX=/opt/open62541 \
    && cmake --build /build -j"$(nproc)" \
    && cmake --install /build

COPY interop/open62541/main.c interop/open62541/client_check.c /app/
RUN mkdir /out \
    && gcc -O2 -Wall -Wextra -I/opt/open62541/include -o /out/opcua-server /app/main.c \
         /opt/open62541/lib/libopen62541.a -lssl -lcrypto -lpthread -lm \
    && gcc -O2 -Wall -Wextra -I/opt/open62541/include -o /out/client_check /app/client_check.c \
         /opt/open62541/lib/libopen62541.a -lssl -lcrypto -lpthread -lm

FROM debian:trixie-slim@sha256:f6b08b09a05725f9431502d3b5714f5c4576e82c1a84fa1debe0c24a7e0c3ec4

ENV DEBIAN_FRONTEND=noninteractive

# openssl is the command line pki.sh makes the keys and certificates with, at container start.
RUN apt-get update \
    && apt-get install -y --no-install-recommends openssl \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /out/opcua-server /out/client_check /usr/local/bin/
COPY interop/open62541/pki.sh interop/open62541/check.sh interop/open62541/serve.sh /usr/local/bin/

# The server needs no privileges: port 4840 is above 1023. /pki is empty in the image; the
# keys are made into it when a container starts and die with the container.
RUN useradd --system --no-create-home --uid 10001 opcua \
    && mkdir /pki \
    && chown 10001 /pki
USER 10001

EXPOSE 4840
ENTRYPOINT ["/usr/local/bin/serve.sh"]
