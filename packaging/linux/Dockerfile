ARG BUILD_IMAGE=debian:trixie
ARG RUNTIME_IMAGE=debian:trixie-slim

FROM ${BUILD_IMAGE} AS build

ARG RUST_VERSION=1.95.0
ARG VERSION=0.0.1
ARG TARGET=x86_64
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential cmake ca-certificates curl git qt6-base-dev \
        protobuf-compiler libprotobuf-dev libnng-dev \
    && rm -rf /var/lib/apt/lists/*

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
      | sh -s -- -y --profile minimal --default-toolchain "${RUST_VERSION}"
ENV PATH=/root/.cargo/bin:$PATH

COPY . /src
WORKDIR /src

RUN cargo build --release --locked --manifest-path rust/Cargo.toml
RUN cmake -S gui -B /build/gui -DCMAKE_BUILD_TYPE=Release \
    && cmake --build /build/gui -j"$(nproc)"

RUN set -eu; \
    name="openmso-${VERSION}-linux-${TARGET}"; \
    stage="/stage/${name}"; \
    mkdir -p "${stage}/bin" "${stage}/plugins/demo" /out; \
    cmake --install /build/gui --prefix "${stage}"; \
    cp rust/target/release/omso-cli "${stage}/bin/"; \
    cp rust/target/release/demo "${stage}/plugins/demo/"; \
    strip "${stage}/bin/omso" "${stage}/bin/omso-cli" "${stage}/plugins/demo/demo"; \
    sed -e 's|"run": \[.*\]|"run": ["./demo"]|' -e '/"build":/d' \
        plugins/demo/plugin.json > "${stage}/plugins/demo/plugin.json"; \
    cp README.md packaging/linux/NOTICE.md "${stage}/"; \
    cp packaging/linux/COPYING "${stage}/COPYING"; \
    cp LICENSE "${stage}/LICENSE.Apache-2.0"; \
    tar -C /stage --owner=0 --group=0 -czf "/out/${name}.tar.gz" "${name}"; \
    cd /out && sha256sum "${name}.tar.gz" > "${name}.tar.gz.sha256"

# Unpacks the tarball with only the runtime packages a user is told to install:
# no toolchain, no -dev packages, no build tree.
FROM ${RUNTIME_IMAGE} AS smoke

ARG VERSION=0.0.1
ARG TARGET=x86_64
ENV DEBIAN_FRONTEND=noninteractive

# Exactly the list NOTICE.md tells a user to install, so a wrong list fails
# the build rather than the user's first run.
RUN apt-get update && apt-get install -y --no-install-recommends \
        libqt6widgets6 libnng1 libprotobuf32t64 qt6-wayland \
    && rm -rf /var/lib/apt/lists/*

COPY --from=build /out/ /tmp/dist/

RUN set -eu; \
    name="openmso-${VERSION}-linux-${TARGET}"; \
    tar -C /opt -xzf "/tmp/dist/${name}.tar.gz"; \
    root="/opt/${name}"; \
    for exe in "${root}/bin/omso" "${root}/bin/omso-cli" \
               "${root}/plugins/demo/demo"; do \
        echo "ldd ${exe}"; \
        if ldd "${exe}" | grep "not found"; then \
            echo "FAIL: unresolved libraries in ${exe}" >&2; exit 1; \
        fi; \
    done; \
    "${root}/bin/omso-cli" --plugin demo --device demo://0 \
        capture -o /tmp/smoke.sr --csv /tmp/smoke.csv; \
    test -s /tmp/smoke.sr; \
    test -s /tmp/smoke.csv; \
    # The binaries are GPL-3.0-or-later as distributed, so the text travels
    # with them; the Apache notice covers the sources they were built from.
    test -s "${root}/COPYING"; \
    test -s "${root}/LICENSE.Apache-2.0"; \
    # No compositor here to run against, so the check is that the package
    # providing the Wayland platform plugin was one a user is told to install.
    test -e /usr/lib/*-linux-gnu/qt6/plugins/platforms/libqwayland-generic.so; \
    # A GUI still alive when timeout fires (124) started cleanly; any other
    # status means it failed to open a display or load its platform plugin.
    QT_QPA_PLATFORM=offscreen timeout 10 "${root}/bin/omso" \
        && { echo "FAIL: GUI exited early" >&2; exit 1; } \
        || test $? -eq 124; \
    touch /smoke-ok

FROM ${RUNTIME_IMAGE} AS artifact
COPY --from=smoke /smoke-ok /smoke-ok
COPY --from=build /out/ /out/
