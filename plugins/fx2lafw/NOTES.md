# fx2lafw (Cypress FX2) behavioral notes

Bench unit: Saleae Logic clone, VID:PID `0925:3881` (Lakeview Research).
Firmware blob: `/usr/share/sigrok-firmware/fx2lafw-saleae-logic.fw`
(sigrok-firmware-fx2lafw 0.1.7-3, 8120 bytes, raw binary loaded at 0x0000;
first bytes `02 01 b9` = 8051 `LJMP 0x01b9` at the reset vector).

## Constants (verified on the bench, 2026-07-22)

- **Cypress bootloader vendor request**: `0xA0`, recipient Device, `wValue` =
  target RAM addr (CPUCS register `0xE600` for reset control). Handled by the
  FX2 ROM while the 8051 is in reset; fx2lafw does NOT implement 0xA0, so never
  blind-upload to a running device.
- **fx2lafw vendor commands** (EP0, recipient Device):
  - `CMD_GET_FW_VERSION = 0xB0` — control IN, returns 2 bytes (major, minor).
    Used as the "is fx2lafw running?" probe. On this Saleae clone the FX2 ROM
    bootloader **ignores** `0xB0` (does not STALL it), so the probe times out
    and nusb returns `TransferError::Cancelled`. The plugin treats
    `Stall`/`Fault`/`Cancelled` as "bootloader" and proceeds to upload.
  - `CMD_GET_REVID_VERSION = 0xB2` — control IN, 1 byte.
  - `CMD_START = 0xB1` — control OUT, 3-byte payload `{flags, delay_h, delay_l}`.
- **CMD_START flags byte**:
  - bit 5 (`WIDE`): 0 = 8-bit / 8 ch, 1 = 16-bit / 16 ch. We use 8-bit.
  - bit 6 (`CLK`): 0 = 30 MHz IFCLK, 1 = 48 MHz IFCLK.
- **Sample-rate → delay**: `delay = (clk / rate) - 1`, big-endian into h/l.
  - All advertised rates (20 kHz … 24 MHz) divide 48 MHz, so `flags = FLAG_CLK_48`.
  - Sanity: 24 MHz → delay 1; 1 MHz → 47; 20 kHz → 2399. All fit u16.
- **Data path**: EP2 bulk IN (`0x82`, 512-byte HS packets), 1 byte/sample,
  bit *i* = channel D*i* — byte-identical to OCP logic encoding `unitsize: 1`.
  Endpoint address is discovered from the active alt-setting's descriptor
  (not hardcoded) so alternate descriptor layouts work.
- **Stop**: no explicit stop command — cancel pending URBs + `clear_halt` on
  EP2 (the documented fx2lafw stop). No hardware trigger; no on-device memory.

## Firmware upload flow (verified on the bench, 2026-07-22)

1. Open device, claim interface 0.
2. `CMD_GET_FW_VERSION` probe: STALL/Fault → bootloader; Ok → already running.
3. Bootloader path:
   1. `control_out(0xA0, value=0xE600, data=[0x01])` — assert 8051 reset.
   2. For each 1024-byte chunk: `control_out(0xA0, value=addr, data=chunk)`
      where `addr` starts at 0x0000 and increments by chunk length.
   3. `control_out(0xA0, value=0xE600, data=[0x00])` — release reset; the
      firmware boots and the device re-enumerates at a (likely) new bus
      address.
   4. Drop the handle, poll `list_devices()` for `0925:3881` (bounded 5 s),
      re-open + re-claim + re-probe. The probe answers once fx2lafw is up.

Re-enumeration typically completes in ≤ 1 s on this bench (Linux 6.x,
usbdevfs). The plugin tolerates the OS reusing the same bus address by
re-probing rather than comparing addresses.

## Cross-check against sigrok-cli

`sigrok-cli --scan` finds the device as `fx2lafw:conn=3.38 - Saleae Logic
[S/N: Saleae Logic] with 8 channels: D0 D1 D2 D3 D4 D5 D6 D7`. A 1 MHz / 100 k
capture matches sigrok-cli `-d fx2lafw --config samplerate=1m --samples 100000
-O bits` on a 1 kHz square wave on D0/D1 within one sample period (the OCP
plugin and sigrok-cli share the same device firmware, so the only divergence
risk is in the host driver — verified to match on the bench).

## nusb 0.2.5 API notes (used by this plugin)

- All async-returns are `impl MaybeFuture`; call `.wait()` to block (no async
  runtime needed). Import the `nusb::MaybeFuture` trait.
- `DeviceInfo`: `vendor_id()`, `product_id()`, `serial_number()`, `busnum()`,
  `device_address()`, `open() -> Result<Device, Error>`.
- `Device::claim_interface(n) -> Interface` (Linux/macOS/Windows).
- **Control transfers**: use `Interface::control_in/control_out` (not
  `Device::control_*`, which is gated to linux/macos/android — the Interface
  path compiles on all three and is required on Windows anyway).
  - `control_in(ControlIn{..}, Duration) -> Result<Vec<u8>, TransferError>`
  - `control_out(ControlOut{..}, Duration) -> Result<(), TransferError>`
- **Bulk IN**: `interface.endpoint::<Bulk, In>(addr) -> Result<Endpoint<Bulk, In>, Error>`.
  - Simple: `ep.transfer_blocking(Buffer, Duration) -> Completion`.
  - Queued (best throughput, multiple in-flight URBs): `ep.allocate(len)`,
    `ep.submit(buf)`, `ep.wait_next_complete(timeout) -> Option<Completion>`,
    `ep.cancel_all()`, `ep.clear_halt() -> Result<(), Error>`.
  - `EndpointRead` (via `ep.reader(buf_size)`) implements `std::io::Read` /
    `BufRead` for a higher-level streaming API.
- **`Completion`** (pub fields): `buffer: Buffer`, `actual_len: usize`,
  `status: Result<(), TransferError>`. `Buffer::into_vec()` is zero-cost for
  the default allocator; driver updates `Buffer::len` to received bytes on IN
  completion, so `into_vec()` already has the right length.
- **`TransferError` variants**: `Cancelled | Stall | Disconnected | Fault |
  InvalidArgument | Unknown(u32)`. `Stall` is the firmware-present probe signal.
