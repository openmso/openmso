# OpenMSO

Pluggable, open source front-end for mixed-signal / digital storage oscilloscopes and logic analyzers.

Capture runs in separate **plugin processes** that speak the [OpenMSO Capture Protocol](docs/protocol.md) (OCP) over stdio or TCP - similar to how LSP or DAP work - so that anyone can write a capture plugin for their hardware, without necessitating changes to this repo.

Note that this is currently a prototype / work in progress made with LLM assistance. See [LICENSES.md](LICENSES.md) for the licensing model (Apache-2.0 core, GPL only where sigrok code is reused).

## Layout

```
docs/protocol.md      OCP wire protocol specification
python/openmso/       Python reference implementation (framing, RPC, srzip
                      writer, SCPI-over-TCP/USBTMC/VXI-11 transports)
rust/openmso-plugin/  Rust plugin-side library (framing, serve loop, SCPI
                      transports incl. a dependency-free VXI-11 client)
rust/sds1000xe/       Siglent SDS1000X-E capture plugin (native binary)
plugins/sds1000xe/    plugin manifest, udev rule, hardware notes
plugins/demo/         simulated mixed-signal device (no hardware needed)
cli/omso              command-line frontend
tests/                unit tests (Rust ones live next to the code: cargo test)
```

## Quick start

```sh
# Build the native plugins once (needs a Rust toolchain):
cargo build --release --manifest-path rust/Cargo.toml

# Simulated device, no hardware:
./cli/omso --plugin demo capture -o demo.sr --csv demo.csv
pulseview demo.sr

# Siglent SDS1000X-E over Ethernet:
./cli/omso --plugin sds1000xe --address 192.168.1.155 scan
./cli/omso --plugin sds1000xe --address 192.168.1.155 capture \
    --channels C1,C3 --mode single -o capture.sr

# Over USB (install plugins/sds1000xe/99-openmso-usbtmc.rules first):
./cli/omso --plugin sds1000xe scan
```

Captures are written as sigrok-compatible `.sr` files and open in PulseView.

