# Notice

Built and tested on Debian 13 (trixie), x86-64.

## Install

    sudo apt install libqt6widgets6 libnng1 libprotobuf32t64 qt6-wayland

    tar -xzf openmso-<version>-linux-x86_64.tar.gz
    cd openmso-<version>-linux-x86_64
    ./bin/omso
    ./bin/omso-cli --plugin demo --device demo://0 capture -o demo.sr

Only `omso` needs those packages; `omso-cli` and the `demo` plugin need glibc
alone. Without `qt6-wayland`, Qt falls back to XWayland.

## Licensing

This binary distribution is GPL-3.0-or-later as a whole; see `COPYING`.
OpenMSO's own sources remain Apache-2.0, in `LICENSE.Apache-2.0`.

Qt 6 (LGPL-3.0), nng (MIT) and Protocol Buffers (BSD-3-Clause) are linked
against, not included; `apt source <package>` for their sources.
