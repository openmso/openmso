# SPDX-License-Identifier: Apache-2.0
"""Minimal VXI-11 (TCP/IP Instrument Protocol) client.

Implements just enough ONC-RPC (RFC 1057/5531) and VXI-11 (VXIbus TC
specification) to drive an instrument: portmapper GETPORT, create_link,
device_write, device_read, destroy_link. Pure stdlib, written from the public
specifications.

Practical motivation: Siglent SDS1000X-E raw-socket SCPI (port 5025) is
fragile and can crash, taking 5024/5025 down until reboot; the VXI-11
service is a separate, more robust path and is what most vendor software uses.
"""

import random
import socket
import struct

PMAP_PROG, PMAP_VERS, PMAP_GETPORT = 100000, 2, 3
CORE_PROG, CORE_VERS = 395183, 1
PROC_CREATE_LINK, PROC_DEV_WRITE, PROC_DEV_READ = 10, 11, 12
PROC_DESTROY_LINK = 23
IPPROTO_TCP = 6

# device_read 'reason' bits
REASON_REQCNT = 1     # requestSize bytes read
REASON_CHR = 2        # termchar seen
REASON_END = 4        # END indicator (end of message)

WRITE_FLAG_END = 8


class Vxi11Error(Exception):
    pass


def _opaque(data):
    n = len(data)
    return struct.pack(">I", n) + data + b"\x00" * ((4 - n % 4) % 4)


class _RpcChannel:
    """One ONC-RPC connection with record-marking framing."""

    def __init__(self, host, port, timeout):
        self.sock = socket.create_connection((host, port), timeout=timeout)

    def call(self, prog, vers, proc, args):
        xid = random.getrandbits(31)
        hdr = struct.pack(">IIIIII", xid, 0, 2, prog, vers, proc)
        cred_verf = struct.pack(">IIII", 0, 0, 0, 0)  # AUTH_NULL cred + verf
        record = hdr + cred_verf + args
        self.sock.sendall(struct.pack(">I", 0x80000000 | len(record)) + record)
        reply = self._read_record()
        (rxid, mtype, rstat) = struct.unpack(">III", reply[:12])
        if rxid != xid or mtype != 1:
            raise Vxi11Error(f"bad RPC reply (xid {rxid:#x} vs {xid:#x})")
        if rstat != 0:
            raise Vxi11Error(f"RPC call rejected (stat {rstat})")
        # skip verf (flavor + length + body), then accept_stat
        vlen = struct.unpack(">I", reply[16:20])[0]
        off = 20 + vlen + ((4 - vlen % 4) % 4)
        astat = struct.unpack(">I", reply[off:off + 4])[0]
        if astat != 0:
            raise Vxi11Error(f"RPC accept_stat {astat}")
        return reply[off + 4:]

    def _read_record(self):
        fragments = []
        last = False
        while not last:
            head = self._read_exactly(4)
            (mark,) = struct.unpack(">I", head)
            last = bool(mark & 0x80000000)
            fragments.append(self._read_exactly(mark & 0x7FFFFFFF))
        return b"".join(fragments)

    def _read_exactly(self, n):
        buf = b""
        while len(buf) < n:
            chunk = self.sock.recv(n - len(buf))
            if not chunk:
                raise Vxi11Error("connection closed by instrument")
            buf += chunk
        return buf

    def close(self):
        try:
            self.sock.close()
        except OSError:
            pass


class Vxi11Client:
    """A single VXI-11 instrument link (device "inst0")."""

    def __init__(self, host, device=b"inst0", timeout=10.0):
        self.host = host
        self.timeout = timeout
        self.io_timeout_ms = int(timeout * 1000)
        pmap = _RpcChannel(host, 111, timeout)
        try:
            args = struct.pack(">IIII", CORE_PROG, CORE_VERS, IPPROTO_TCP, 0)
            r = pmap.call(PMAP_PROG, PMAP_VERS, PMAP_GETPORT, args)
            (port,) = struct.unpack(">I", r[:4])
        finally:
            pmap.close()
        if port == 0:
            raise Vxi11Error("instrument does not register VXI-11 core channel")
        self._chan = _RpcChannel(host, port, timeout)
        # create_link args: clientId(int), lockDevice(bool), lock_timeout(u32)
        args = struct.pack(">iII", 1, 0, 0) + _opaque(device)
        r = self._chan.call(CORE_PROG, CORE_VERS, PROC_CREATE_LINK, args)
        err, self._lid, self._abort_port, self._max_recv = \
            struct.unpack(">IIII", r[:16])
        if err != 0:
            raise Vxi11Error(f"create_link failed (error {err})")

    def write(self, data):
        if isinstance(data, str):
            data = data.encode()
        max_chunk = max(1024, self._max_recv or 4096)
        off = 0
        while off < len(data) or not data:
            chunk = data[off:off + max_chunk]
            off += len(chunk)
            end = WRITE_FLAG_END if off >= len(data) else 0
            args = struct.pack(">IIII", self._lid, self.io_timeout_ms, 0,
                               end) + _opaque(chunk)
            r = self._chan.call(CORE_PROG, CORE_VERS, PROC_DEV_WRITE, args)
            (err,) = struct.unpack(">I", r[:4])
            if err != 0:
                raise Vxi11Error(f"device_write failed (error {err})")
            if not data:
                break

    def read(self, request_size=1 << 20, io_timeout_ms=None):
        """Read one complete message (until END indicator)."""
        out = []
        while True:
            args = struct.pack(">IIIIIi", self._lid, request_size,
                               io_timeout_ms or self.io_timeout_ms, 0, 0, 0)
            r = self._chan.call(CORE_PROG, CORE_VERS, PROC_DEV_READ, args)
            err, reason = struct.unpack(">II", r[:8])
            if err != 0:
                raise Vxi11Error(f"device_read failed (error {err})")
            (dlen,) = struct.unpack(">I", r[8:12])
            out.append(r[12:12 + dlen])
            if reason & (REASON_END | REASON_CHR):
                break
            if reason == 0 and not dlen:
                raise Vxi11Error("device_read returned no data, no reason")
        return b"".join(out)

    def ask(self, cmd, request_size=1 << 20):
        self.write(cmd)
        return self.read(request_size)

    def close(self):
        try:
            args = struct.pack(">I", self._lid)
            self._chan.call(CORE_PROG, CORE_VERS, PROC_DESTROY_LINK, args)
        except Vxi11Error:
            pass
        self._chan.close()


class Vxi11Scpi:
    """SCPI transport over VXI-11, matching the TcpScpi/UsbTmcScpi interface."""

    def __init__(self, host, timeout=10.0):
        self.host = host
        self._cli = Vxi11Client(host, timeout=timeout)

    def command(self, cmd):
        self._cli.write(cmd + "\n")

    def query(self, cmd, timeout=None):
        io_ms = int(timeout * 1000) if timeout else None
        self._cli.write(cmd + "\n")
        return self._cli.read(65536, io_timeout_ms=io_ms) \
            .decode(errors="replace").strip()

    def query_block(self, cmd, timeout=None):
        from .scpi import _find_block, ScpiError
        io_ms = int(timeout * 1000) if timeout else None
        self._cli.write(cmd + "\n")
        buf = self._cli.read(1 << 22, io_timeout_ms=io_ms)
        loc = _find_block(buf)
        if loc is None:
            raise ScpiError(f"incomplete block reply for {cmd!r}")
        start, dlen = loc
        if len(buf) < start + dlen:
            raise ScpiError(f"short block: have {len(buf) - start}/{dlen}")
        return buf[start:start + dlen]

    def close(self):
        self._cli.close()
