# Siglent SDS-1000X-E [WIP]

The `siglent-sds1000xe` capture plugin allows capturing from SDS1000X-E series oscilloscopes.

It is currently a **work in progress**. Support for the SLA1016 Logic Analyzer attachment has not been implemented.

## Capture plugin scope

Initial scope is the SDS1000X-E series, which is 4 closely-related devices. Hardware is available for only one of these.

| Board       | Status         |
|-------------|----------------|
| SDS-1102X-E | Planned        |
| SDS-1104X-E | In Development |
| SDS-1202X-E | Planned        |
| SDS-1204X-E | Planned        |

The same programming guide also applies to the following devices. If a developer has access to any of this hardware
and wants to add support, it could possibly be implemented in this plugin as well.

- SDS1000CML/CML+
- SDS1000DL/DL+
- SDS1000CNL/CNL+
- SDS1000/1000X/1000X-S/1000X+/1000X-E
- SDS2000/SDS2000X

## Reference docs

- [Siglent Digital Oscilloscopes Series Programming Guide (RC01020-E01C)](https://web.archive.org/web/20251024015247/https://www.batronix.com/files/Siglent/Oszilloskope/SIGLENT_Digital_Oscilloscopes_RemoteControlManual.pdf)
