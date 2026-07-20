# OpenMSO licensing policy

OpenMSO uses a hybrid licensing model. The core libraries are permissively licensed to encourage re-use, while two specific components are planned to be licensed GPL 3.0 or later in order to allow re-use of sigrok code. A process boundary exists between copyleft and non-copyleft code.

| Component | License | Why |
|---|---|---|
| Protocol specification (`docs/protocol.md`) | Apache-2.0 | Anyone — including proprietary vendors — may implement OCP in frontends, plugins, or firmware. |
| `python/openmso` (framing, RPC, client/server helpers, srzip writer, SCPI transports) | Apache-2.0 | Reference implementation; free to embed anywhere. |
| `rust/openmso-plugin` (plugin-side framing, serve loop, SCPI/VXI-11/usbtmc transports) | Apache-2.0 | Library for native plugins; free to embed anywhere. |
| Official capture plugins (`plugins/*`, `rust/sds1000xe`) | Apache-2.0 |
| A capture plugin which wraps libsigrok (`plugins/libsigrok`, not currentlyi implemented) | GPL-3.0-or-later | 
| CLI frontend (`cli/`) | Apache-2.0 | |
| GUI (`gui/`) | GPL-3.0-or-later | Likely to use parts of PulseView and libsigrokdecode (GPLv3+). |

Rules for contributors:

- GPL components live in clearly marked directories and communicate with the rest of the system only via the OCP wire protocol.
- Third-party firmware blobs (e.g. fx2lafw, GPLv2+) may be redistributed alongside plugins under their own license; they are separate programs loaded onto devices, not linked code.
- Pull requests must include a note clarifying the authorship and licensing status of the code.

`LICENSE-APACHE` contains the Apache-2.0 text. GPL components will carry their own COPYING files in their subdirectories.
