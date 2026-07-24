# SPDX-License-Identifier: Apache-2.0
"""Unit tests: OCP framing codec and srzip writer layout."""

import io
import os
import sys
import zipfile

import numpy as np
import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "python"))

from openmso.framing import MessageStream, ProtocolError
from openmso.srzip import SrZipWriter, samplerate_string


def roundtrip(messages):
    buf = io.BytesIO()
    out = MessageStream(io.BytesIO(), buf)
    for msg, payload in messages:
        out.write_message(msg, payload)
    inp = MessageStream(io.BytesIO(buf.getvalue()), io.BytesIO())
    got = []
    while (item := inp.read_message()) is not None:
        got.append(item)
    return got


def test_plain_messages():
    got = roundtrip([({"a": 1}, None), ({"b": [1, 2]}, None)])
    assert got == [({"a": 1}, None), ({"b": [1, 2]}, None)]


def test_binary_payload():
    payload = bytes(range(256)) * 100
    got = roundtrip([({"method": "capture.data"}, payload), ({"end": True}, None)])
    assert got[0][0]["binlen"] == len(payload)
    assert got[0][1] == payload
    assert got[1] == ({"end": True}, None)


def test_payload_containing_newlines():
    payload = b"\n\n{\"fake\":1}\n" * 50
    got = roundtrip([({"m": 1}, payload), ({"m": 2}, None)])
    assert got[0][1] == payload
    assert got[1][0]["m"] == 2


def test_eof_inside_payload_raises():
    buf = io.BytesIO()
    out = MessageStream(io.BytesIO(), buf)
    out.write_message({"m": 1}, b"x" * 100)
    truncated = buf.getvalue()[:-40]
    inp = MessageStream(io.BytesIO(truncated), io.BytesIO())
    with pytest.raises(ProtocolError):
        inp.read_message()


def test_bad_json_raises():
    inp = MessageStream(io.BytesIO(b"{nope}\n"), io.BytesIO())
    with pytest.raises(ProtocolError):
        inp.read_message()


def test_samplerate_string():
    assert samplerate_string(1e9) == "1 GHz"
    assert samplerate_string(500e6) == "500 MHz"
    assert samplerate_string(1e3) == "1 kHz"
    assert samplerate_string(12500) == "12500 Hz"


def test_srzip_layout(tmp_path):
    path = str(tmp_path / "t.sr")
    w = SrZipWriter(path, 1_000_000, logic_channels=["D0", "D1"],
                    analog_channels=["C1"])
    w.add_logic(b"\x00\x01\x02\x03")
    w.add_analog(0, np.array([0.0, 1.5, -1.5], dtype=np.float32))
    w.close()

    with zipfile.ZipFile(path) as z:
        names = set(z.namelist())
        assert {"version", "metadata", "logic-1-1", "analog-1-3-1"} <= names
        assert z.read("version") == b"2"
        meta = z.read("metadata").decode()
        assert "total probes = 2" in meta
        assert "samplerate = 1 MHz" in meta
        assert "unitsize = 1" in meta
        assert "probe1 = D0" in meta
        assert "analog3 = C1" in meta          # numbering starts after probes
        assert z.read("logic-1-1") == b"\x00\x01\x02\x03"
        vals = np.frombuffer(z.read("analog-1-3-1"), dtype=np.float32)
        assert list(vals) == [0.0, 1.5, -1.5]


def test_srzip_analog_only(tmp_path):
    path = str(tmp_path / "a.sr")
    w = SrZipWriter(path, 1_000_000_000, analog_channels=["C1", "C3"])
    w.add_analog(0, np.zeros(10, dtype=np.float32))
    w.add_analog(1, np.ones(10, dtype=np.float32))
    w.close()
    with zipfile.ZipFile(path) as z:
        meta = z.read("metadata").decode()
        assert "total analog = 2" in meta
        assert "capturefile" not in meta
        assert "analog1 = C1" in meta and "analog2 = C3" in meta
        assert {"analog-1-1-1", "analog-1-2-1"} <= set(z.namelist())
