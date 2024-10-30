from dataclasses import dataclass
from enum import Enum

from .bytes_reader import BytesReader


class Asn1TagClass(Enum):
    UNIVERSAL = 0
    APPLICATION = 1
    CONTEXT_SPECIFIC = 2
    PRIVATE = 3


class Asn1TagType(Enum):
    SEQUENCE = 16


@dataclass
class Asn1Tag:
    num: int
    pc: bool
    cls: Asn1TagClass

    def is_universal(self) -> bool:
        return self.cls == Asn1TagClass.UNIVERSAL

    def is_context_specific(self) -> bool:
        return self.cls == Asn1TagClass.CONTEXT_SPECIFIC

    def is_application(self) -> bool:
        return self.cls == Asn1TagClass.APPLICATION


def read_tag(reader: BytesReader) -> Asn1Tag:
    first_byte = reader.read_u8()

    cls = Asn1TagClass((first_byte & 0b11000000) >> 6)
    pc = bool(first_byte & 0b00100000)
    num = first_byte & 0b00011111

    if num == 31:
        num = 0
        while (reader.peek_u8() & 0b10000000) != 0:
            num <<= 7
            num |= reader.read_u8() & 0b01111111

    return Asn1Tag(num, pc, cls)


def read_length(reader: BytesReader) -> int:
    first_byte = reader.read_u8()

    if (first_byte & 0b10000000) == 0:
        # Definite form, short
        return first_byte
    elif first_byte == 0b10000000:
        # Indefinite form (not supported)
        raise RuntimeError("ASN.1 indefinite length is not supported")
    elif first_byte == 0b11111111:
        # Reserved
        raise RuntimeError("Encountered reserved length encoding")
    else:
        # Definite form, long
        length = 0
        for _ in range(first_byte & 0b01111111):
            length <<= 8
            length |= reader.read_u8()
        return length


def read_integer(reader: BytesReader, length: int) -> int:
    contents = reader.read_bytes(length)

    value = 0
    for byte in contents:
        value <<= 8
        value |= byte

    return value


def read_octet_string(reader: BytesReader, tag: Asn1Tag, length: int) -> bytes:
    if tag.pc:
        raise NotImplementedError("Constructed strings are not supported")
    else:
        return reader.read_bytes(length)


def read_visiblestring(reader: BytesReader, tag: Asn1Tag, length: int) -> str:
    if tag.pc:
        raise NotImplementedError("Constructed strings are not supported")
    else:
        # TODO: Validate character set (for VisibleString it is ASCII 0x20 to 0x7E)
        return str(reader.read_bytes(length), "ascii")
