from __future__ import annotations

import struct


class BytesReader:
    def __init__(self, buffer: bytes):
        self._buffer = buffer

    def sub_reader(self, length: int) -> BytesReader:
        return BytesReader(self._buffer[:length])

    def skip(self, offset: int) -> None:
        self._buffer = self._buffer[offset:]

    def read_u8(self) -> int:
        value = self.peek_u8()
        self.skip(1)
        return value

    def read_u16_be(self) -> int:
        (value,) = struct.unpack_from(">H", self._buffer)
        self.skip(2)
        return int(value)

    def read_bytes(self, length: int) -> bytes:
        value = self._buffer[:length]
        self.skip(length)
        return value

    def peek_u8(self) -> int:
        return self._buffer[0]
