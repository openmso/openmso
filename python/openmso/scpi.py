# SPDX-License-Identifier: Apache-2.0
"""Device-side SCPI transports: raw TCP sockets and Linux usbtmc character
devices. Written from scratch (libsigrok's src/scpi/ served as a behavioral
reference only).

IEEE 488.2 definite-length block parsing handles the `#<n><len><data>` form
used by waveform queries, tolerant of a text prefix before the `#` (Siglent
prepends e.g. ``C1:WF DAT2,``).
"""

import os
import socket


class ScpiError(Exception):
    pass


def _find_block(buf):
    """Locate a definite-length block header in buf.

    Returns (data_start, data_len) if the full header is present, or None if
    more bytes are needed. Raises ScpiError if there is no '#' in a
    reasonable prefix.
    """
    i = buf.find(b"#")
    if i < 0:
        if len(buf) > 64:
            raise ScpiError(f"no block header in response: {buf[:64]!r}")
        return None
    if len(buf) < i + 2:
        return None
    ndigits = buf[i + 1:i + 2]
    if not ndigits.isdigit():
        raise ScpiError(f"bad block header at {buf[i:i+8]!r}")
    ndigits = int(ndigits)
    if len(buf) < i + 2 + ndigits:
        return None
    dlen = int(buf[i + 2:i + 2 + ndigits])
    return i + 2 + ndigits, dlen


class TcpScpi:
    """SCPI over a raw TCP socket (e.g. Siglent port 5025)."""

    def __init__(self, host, port=5025, timeout=5.0):
        self.host, self.port = host, port
        self._sock = socket.create_connection((host, port), timeout=timeout)
        self._timeout = timeout

    def command(self, cmd):
        self._sock.sendall(cmd.encode() + b"\n")

    def query(self, cmd, timeout=None):
        self.command(cmd)
        self._sock.settimeout(timeout or self._timeout)
        buf = b""
        while not buf.endswith(b"\n"):
            chunk = self._sock.recv(4096)
            if not chunk:
                raise ScpiError(f"connection closed during query {cmd!r}")
            buf += chunk
        return buf.decode(errors="replace").strip()

    def query_block(self, cmd, timeout=None):
        """Send a query returning a definite-length block; return its bytes."""
        self.command(cmd)
        self._sock.settimeout(timeout or self._timeout)
        buf = b""
        loc = None
        while loc is None:
            chunk = self._sock.recv(65536)
            if not chunk:
                raise ScpiError(f"connection closed reading block for {cmd!r}")
            buf += chunk
            loc = _find_block(buf)
        start, dlen = loc
        total = start + dlen
        parts = [buf]
        got = len(buf)
        while got < total:
            chunk = self._sock.recv(1 << 20)
            if not chunk:
                raise ScpiError(
                    f"connection closed mid-block ({got}/{total} bytes)")
            parts.append(chunk)
            got += len(chunk)
        buf = b"".join(parts)
        data = buf[start:total]
        self._drain_terminator(len(buf) - total)
        return data

    def _drain_terminator(self, already):
        # Siglent terminates blocks with "\n\n"; consume what wasn't already read.
        want = max(0, 2 - already)
        if not want:
            return
        self._sock.settimeout(0.5)
        try:
            self._sock.recv(want)
        except (TimeoutError, socket.timeout):
            pass
        finally:
            self._sock.settimeout(self._timeout)

    def close(self):
        try:
            self._sock.close()
        except OSError:
            pass


class UsbTmcScpi:
    """SCPI over a Linux /dev/usbtmc* character device.

    The kernel usbtmc driver handles USB-TMC framing; reads return message
    chunks (a reply may span several reads). Requires read/write access to
    the device node (udev rule).
    """

    READ_CHUNK = 1 << 20
    USBDEVFS_RESET = (ord("U") << 8) | 20
    VENDOR_IDS = (0xF4EC,)   # Siglent; extend as other usbtmc gear is tested

    def __init__(self, path):
        self.path = path
        self._fd = os.open(path, os.O_RDWR)
        self._recovered = False

    def command(self, cmd):
        os.write(self._fd, cmd.encode() + b"\n")

    def _read_chunk(self, n):
        """One usbtmc read. A 0-byte result, ETIMEDOUT or EPIPE means the
        interface is stuck (a previous session closed with undrained reply
        data). Recover once per session via USB device reset + reopen."""
        import errno
        try:
            chunk = os.read(self._fd, n)
        except OSError as e:
            if e.errno not in (errno.ETIMEDOUT, errno.EPIPE) or self._recovered:
                raise
            chunk = b""
        if chunk or self._recovered:
            return chunk
        self._recovered = True
        self._usb_reset()
        return b""

    def _usb_reset(self):
        import fcntl
        import glob as _glob
        os.close(self._fd)
        self._fd = None
        for dev in _glob.glob("/dev/bus/usb/*/*"):
            try:
                with open(dev, "rb") as f:
                    desc = f.read(18)
                vid = int.from_bytes(desc[8:10], "little")
                if vid in self.VENDOR_IDS:
                    fd = os.open(dev, os.O_WRONLY)
                    fcntl.ioctl(fd, self.USBDEVFS_RESET, 0)
                    os.close(fd)
                    break
            except OSError:
                continue
        else:
            raise ScpiError(f"{self.path}: interface stuck and no USB device "
                            f"found to reset")
        import time
        time.sleep(1.0)
        self._fd = os.open(self.path, os.O_RDWR)

    def query(self, cmd, timeout=None):
        buf = b""
        for _ in range(2):   # retry once after an automatic recovery
            self.command(cmd)
            buf = b""
            # Loop until we have real content ending in LF (tolerates stray
            # terminator bytes left over from a previous block transfer).
            while not (buf.strip() and buf.endswith(b"\n")):
                chunk = self._read_chunk(4096)
                if not chunk:
                    break
                buf += chunk
            if buf.strip():
                break
        return buf.decode(errors="replace").strip()

    def query_block(self, cmd, timeout=None):
        for _ in range(2):   # retry once if a stuck interface was recovered
            self.command(cmd)
            buf = self._read_chunk(self.READ_CHUNK)
            if buf:
                break
        loc = None
        while loc is None:
            if not buf:
                raise ScpiError(f"no reply to block query {cmd!r}")
            loc = _find_block(buf)
            if loc is None:
                chunk = os.read(self._fd, self.READ_CHUNK)
                if not chunk:
                    raise ScpiError(f"EOF reading block header for {cmd!r}")
                buf += chunk
        start, dlen = loc
        total = start + dlen
        parts = [buf]
        got = len(buf)
        while got < total:
            chunk = os.read(self._fd, self.READ_CHUNK)
            if not chunk:
                raise ScpiError(f"EOF mid-block ({got}/{total} bytes)")
            parts.append(chunk)
            got += len(chunk)
        buf = b"".join(parts)
        return buf[start:total]

    def close(self):
        try:
            os.close(self._fd)
        except OSError:
            pass


def open_transport(connection):
    """Open a transport from a connection URL.

    ``vxi11://host``, ``tcp://host[:port]`` or ``usbtmc:///dev/usbtmc0``.
    """
    if connection.startswith("vxi11://"):
        from .vxi11 import Vxi11Scpi
        return Vxi11Scpi(connection[len("vxi11://"):])
    if connection.startswith("tcp://"):
        rest = connection[len("tcp://"):]
        host, _, port = rest.partition(":")
        return TcpScpi(host, int(port) if port else 5025)
    if connection.startswith("usbtmc://"):
        return UsbTmcScpi(connection[len("usbtmc://"):])
    raise ScpiError(f"unsupported connection: {connection!r}")
