ARG BUILD_IMAGE=debian:bookworm
ARG RUNTIME_IMAGE=debian:bookworm-slim

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
    cp LICENSE README.md packaging/linux/NOTICE.md "${stage}/"; \
    tar -C /stage --owner=0 --group=0 -czf "/out/${name}.tar.gz" "${name}"; \
    cd /out && sha256sum "${name}.tar.gz" > "${name}.tar.gz.sha256"

# Unpacks the tarball with only the runtime packages a user is told to install:
# no toolchain, no -dev packages, no build tree.
FROM ${RUNTIME_IMAGE} AS smoke

ARG VERSION=0.0.1
ARG TARGET=x86_64
ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
        libqt6widgets6 libqt6concurrent6 qt6-qpa-plugins \
        libnng1 libprotobuf32 \
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
    # A GUI still alive when timeout fires (124) started cleanly; any other
    # status means it failed to open a display or load its platform plugin.
    QT_QPA_PLATFORM=offscreen timeout 10 "${root}/bin/omso" \
        && { echo "FAIL: GUI exited early" >&2; exit 1; } \
        || test $? -eq 124; \
    touch /smoke-ok

FROM ${RUNTIME_IMAGE} AS artifact
COPY --from=smoke /smoke-ok /smoke-ok
COPY --from=build /out/ /out/
