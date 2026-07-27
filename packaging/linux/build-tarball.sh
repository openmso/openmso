#!/usr/bin/env bash
set -euo pipefail

VERSION="${OPENMSO_VERSION:-0.0.1}"
TARGET="${OPENMSO_TARGET:-x86_64}"
NAME="openmso-${VERSION}-linux-${TARGET}"

ENGINE="${CONTAINER_ENGINE:-}"
if [ -z "${ENGINE}" ]; then
    ENGINE=$(command -v docker || command -v podman || true)
fi
if [ -z "${ENGINE}" ]; then
    echo "$0: need docker or podman (or set CONTAINER_ENGINE)" >&2
    exit 1
fi

repo_root=$(cd "$(dirname "$0")/../.." && pwd)

# The context is the directory above the repo so the sibling openmso-api
# checkout comes along; rust/Cargo.toml depends on it by path. Both checkouts
# must therefore keep their default directory names.
if [ "$(basename "${repo_root}")" != "openmso" ] \
   || [ ! -f "${repo_root}/../openmso-api/rust/openmso/Cargo.toml" ]; then
    echo "$0: expected checkouts named openmso/ and openmso-api/ side by side" >&2
    exit 1
fi

out="${repo_root}/dist"
tag="openmso-artifact:${VERSION}"

echo "==> building ${NAME} with $(basename "${ENGINE}")"
"${ENGINE}" build \
    --target artifact \
    --build-arg "VERSION=${VERSION}" \
    --build-arg "TARGET=${TARGET}" \
    -t "${tag}" \
    -f "${repo_root}/packaging/linux/Dockerfile" \
    "${repo_root}/.."

echo "==> extracting to ${out}"
mkdir -p "${out}"
cid=$("${ENGINE}" create "${tag}" /bin/true)
trap '"${ENGINE}" rm -f "${cid}" >/dev/null 2>&1 || true' EXIT
"${ENGINE}" cp "${cid}:/out/${NAME}.tar.gz" "${out}/"
"${ENGINE}" cp "${cid}:/out/${NAME}.tar.gz.sha256" "${out}/"

cd "${out}"
sha256sum -c "${NAME}.tar.gz.sha256"
echo
echo "==> ${out}/${NAME}.tar.gz"
tar -tzf "${NAME}.tar.gz" | sed 's/^/    /'
