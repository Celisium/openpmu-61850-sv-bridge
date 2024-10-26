from dataclasses import dataclass
from enum import Enum
import logging
import socket
import struct
import time

INTERFACE_NAME = "lo"
ETHERTYPE_SV = 0x88BA
MAX_PACKET_LENGTH = 1522
SAMPLE_LENGTH = 32
NS_PER_SEC = 10**9


@dataclass
class Sample:
    current_a: float
    current_b: float
    current_c: float
    current_n: float
    voltage_a: float
    voltage_b: float
    voltage_c: float
    voltage_n: float

    def __init__(self, data: bytes):
        if len(data) != 64:
            raise RuntimeError("Length of sample is not 64")
        values = struct.unpack(">16i", data)
        self.current_a = values[0] * 0.001
        self.current_b = values[2] * 0.001
        self.current_c = values[4] * 0.001
        self.current_n = values[6] * 0.001
        self.voltage_a = values[8] * 0.01
        self.voltage_b = values[10] * 0.01
        self.voltage_c = values[12] * 0.01
        self.voltage_n = values[14] * 0.01


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


@dataclass
class TimeQuality:
    leap_second_known: bool
    clock_failure: bool
    clock_not_synchronised: bool
    time_accuracy: int


@dataclass
class UtcTime:
    secs: float
    quality: TimeQuality


class BytesReader:
    def __init__(self, buffer: bytes):
        self._buffer = buffer

    def sub_reader(self, length: int):
        return BytesReader(self._buffer[:length])

    def skip(self, offset: int):
        self._buffer = self._buffer[offset:]

    def read_u8(self) -> int:
        value = self._buffer[0]
        self.skip(1)
        return value

    def read_u16_be(self) -> int:
        (value,) = struct.unpack_from(">H", self._buffer)
        self.skip(2)
        return value

    def read_asn1_tag(self) -> Asn1Tag:
        first_byte = self.read_u8()

        cls = Asn1TagClass((first_byte & 0b11000000) >> 6)
        pc = bool(first_byte & 0b00100000)
        num = first_byte & 0b00011111

        if num == 31:
            num = 0
            while (self._buffer[0] & 0b10000000) != 0:
                num <<= 7
                num |= self._buffer[0] & 0b01111111
                self.skip(1)

        return Asn1Tag(num, pc, cls)

    def read_asn1_length(self) -> int | None:
        first_byte = self.read_u8()

        if (first_byte & 0b10000000) == 0:
            # Definite form, short
            return first_byte
        elif first_byte == 0b10000000:
            # Indefinite form
            return None
        elif first_byte == 0b11111111:
            # Reserved
            raise RuntimeError("Encountered reserved length encoding")
        else:
            # Definite form, long
            length = 0
            for _ in range(first_byte & 0b01111111):
                length <<= 8
                length |= self.read_u8()
            return length

    def read_definite_asn1_length(self) -> int:
        length = self.read_asn1_length()
        if length is None:
            raise RuntimeError(
                "Expected a definite length but encountered an indefinite length"
            )
        else:
            return length

    def read_asn1_integer(self, length: int) -> int:
        value = 0
        for i in range(length):
            value <<= 8
            value |= self._buffer[i]
        self.skip(length)
        return value

    def read_asn1_octet_string(self, tag: Asn1Tag, length: int) -> bytes:
        if tag.pc:
            raise NotImplementedError("Constructed strings are not yet supported")
        else:
            string = self._buffer[:length]
            self.skip(length)
        return string

    def read_asn1_visiblestring(self, tag: Asn1Tag, length: int) -> str:
        if tag.pc:
            raise NotImplementedError("Constructed strings are not yet supported")
        else:
            # TODO: Validate character set (for VisibleString it is ASCII 0x20 to 0x7E)
            string = str(self._buffer[:length], "ascii")
            self.skip(length)
        return string

    def read_iec61850_utctime(self) -> UtcTime:
        # ssssssssssssssssssssssssssssssss ffffffffffffffffffffffff qqqqqqqq
        (secs, frac_qual) = struct.unpack_from(">II", self._buffer)
        secs += (frac_qual >> 8) / (1 << 24)

        quality = TimeQuality(
            (frac_qual & 0b10000000) != 0,
            (frac_qual & 0b01000000) != 0,
            (frac_qual & 0b00100000) != 0,
            frac_qual & 0b00011111,  # TODO: Use separate values for invalid/unspecified
        )

        self.skip(8)

        return UtcTime(secs, quality)

    def read_9_2_le_sample(self) -> Sample:
        sample = Sample(self._buffer[:64])
        self.skip(64)
        return sample


@dataclass
class Asdu:
    svid: str
    datset: str | None
    smp_cnt: int
    conf_rev: int
    refr_tm: UtcTime | None
    smp_synch: int
    smp_rate: int | None
    sample: Sample


def read_asdu(reader: BytesReader) -> Asdu:
    # svID [0] IMPLICIT VisibleString
    tag = reader.read_asn1_tag()
    if tag.num == 0 and tag.is_context_specific():
        length = reader.read_definite_asn1_length()
        svid = reader.sub_reader(length).read_asn1_visiblestring(tag, length)
        reader.skip(length)
    else:
        raise RuntimeError("Expected context-specific tag with tag number 0")

    # datset [1] IMPLICIT VisibleString OPTIONAL
    tag = reader.read_asn1_tag()
    if tag.num == 1 and tag.is_context_specific():
        length = reader.read_definite_asn1_length()
        datset = reader.sub_reader(length).read_asn1_visiblestring(tag, length)
        reader.skip(length)
        tag = reader.read_asn1_tag()
    else:
        datset = None

    # smpCnt [2] IMPLICIT OCTET STRING (SIZE(2))
    if tag.num == 2 and tag.is_context_specific():
        length = reader.read_definite_asn1_length()
        if length != 2:
            raise RuntimeError("Expected octet string with length 2")
        smp_cnt_bytes = reader.sub_reader(length).read_asn1_octet_string(tag, length)
        reader.skip(length)
        (smp_cnt,) = struct.unpack(">H", smp_cnt_bytes)
    else:
        raise RuntimeError("Expected context-specific tag with tag number 2")

    # confRev [3] IMPLICIT OCTET STRING (SIZE(4))
    tag = reader.read_asn1_tag()
    if tag.num == 3 and tag.is_context_specific():
        length = reader.read_definite_asn1_length()
        if length != 4:
            raise RuntimeError("Expected octet string with length 4")
        conf_rev_bytes = reader.sub_reader(length).read_asn1_octet_string(tag, length)
        reader.skip(length)
        (conf_rev,) = struct.unpack(">I", conf_rev_bytes)
    else:
        raise RuntimeError("Expected context-specific tag with tag number 3")

    # refrTm [4] IMPLICIT UtcTime OPTIONAL
    # (This is not the universal ASN.1 UTCTime type, but the IEC 61850 UtcTime type.)
    tag = reader.read_asn1_tag()
    if tag.num == 4 and tag.is_context_specific():
        length = reader.read_definite_asn1_length()
        refr_tm = reader.sub_reader(length).read_iec61850_utctime()
        reader.skip(length)
        tag = reader.read_asn1_tag()
    else:
        refr_tm = None

    # smpSynch [5] IMPLICIT OCTET STRING (SIZE(1))
    if tag.num == 5 and tag.is_context_specific():
        length = reader.read_definite_asn1_length()
        if length != 1:
            raise RuntimeError("Expected octet string with length 1")
        smp_synch = reader.read_u8()
    else:
        raise RuntimeError("Expected context-specific tag with tag number 5")

    # smpRate [6] IMPLICIT OCTET STRING (SIZE(2)) OPTIONAL
    tag = reader.read_asn1_tag()
    if tag.num == 6 and tag.is_context_specific():
        length = reader.read_definite_asn1_length()
        if length != 2:
            raise RuntimeError("Expected octet string with length 2")
        smp_rate_bytes = reader.sub_reader(length).read_asn1_octet_string(tag, length)
        reader.skip(length)
        (smp_rate,) = struct.unpack(">H", smp_rate_bytes)
        tag = reader.read_asn1_tag()
    else:
        smp_rate = None

    # sample [7] IMPLICIT OCTET STRING (SIZE(n))
    if tag.num == 7 and tag.is_context_specific():
        length = reader.read_definite_asn1_length()
        sample = reader.read_9_2_le_sample()
    else:
        raise RuntimeError("Expected context-specific tag with tag number 7")

    # TODO: Two more optional values

    return Asdu(svid, datset, smp_cnt, conf_rev, refr_tm, smp_synch, smp_rate, sample)


@dataclass
class SavPdu:
    asdus: list[Asdu]


def read_savpdu(reader: BytesReader) -> SavPdu:
    # noASDU [0] IMPLICIT INTEGER (1..65535)
    tag = reader.read_asn1_tag()
    if tag.num == 0 and tag.is_context_specific():
        length = reader.read_definite_asn1_length()
        no_asdu = reader.sub_reader(length).read_asn1_integer(length)
        reader.skip(length)
        if not 1 <= no_asdu < 65536:
            raise RuntimeError("noASDU out of range")
    else:
        raise RuntimeError("Expected context-specific tag with tag number 0")

    # security [1] ANY OPTIONAL
    tag = reader.read_asn1_tag()
    if tag.num == 1 and tag.is_context_specific():
        length = reader.read_definite_asn1_length()
        reader.skip(length)
        tag = reader.read_asn1_tag()

    # asdu [2] IMPLICIT SEQUENCE OF ASDU
    if tag.num == 2 and tag.is_context_specific():
        length = reader.read_definite_asn1_length()
        inner_reader = reader.sub_reader(length)

        asdus = []
        for _ in range(no_asdu):
            tag = inner_reader.read_asn1_tag()
            if tag.num == 16 and tag.is_universal():
                length = inner_reader.read_definite_asn1_length()
                asdu = read_asdu(inner_reader.sub_reader(length))
                asdus.append(asdu)
                inner_reader.skip(length)
            else:
                raise RuntimeError("Expected universal tag with tag number 16")

    else:
        raise RuntimeError("Expected context-specific tag with tag number 2")

    return SavPdu(asdus)


def read_sv(reader: BytesReader) -> SavPdu:
    tag = reader.read_asn1_tag()
    if tag.num == 0 and tag.is_application():
        length = reader.read_definite_asn1_length()
        savpdu = read_savpdu(reader.sub_reader(length))
        reader.skip(length)
    else:
        raise RuntimeError("Unexpected tag number/class")
    return savpdu


class SampleBuffer:
    def __init__(self, sample_rate: int):
        self._sample_rate = sample_rate
        self._buffer = bytearray(sample_rate * 32)
        self._buffer_start_time_s = 0

    def add_sample(self, recv_time_ns: int, asdu: Asdu):
        ns_per_sample = NS_PER_SEC / (self._sample_rate)
        ns_offset = asdu.smp_cnt * ns_per_sample

        recv_time_s = recv_time_ns // NS_PER_SEC

        if ns_offset >= recv_time_ns % NS_PER_SEC:
            recv_time_s -= 1

        if recv_time_s > self._buffer_start_time_s:
            self._buffer_start_time_s = recv_time_s

            count = 0
            for (sample,) in struct.iter_unpack("=f", self._buffer):
                if sample != 0:
                    count += 1

            print("flush buffer with {} non-zero samples".format(count / 8))

            self._buffer = bytearray(len(self._buffer))

        struct.pack_into(
            "=8f",
            self._buffer,
            asdu.smp_cnt * 32,
            asdu.sample.current_a,
            asdu.sample.current_b,
            asdu.sample.current_c,
            asdu.sample.current_n,
            asdu.sample.voltage_a,
            asdu.sample.voltage_b,
            asdu.sample.voltage_c,
            asdu.sample.voltage_n,
        )


def main():
    logger = logging.getLogger()
    logging.basicConfig(level=logging.DEBUG)

    with socket.socket(socket.AF_PACKET, socket.SOCK_DGRAM, ETHERTYPE_SV) as skt:
        skt.bind((INTERFACE_NAME, ETHERTYPE_SV))

        logger.info("Successfully bound socket to interface '%s'", INTERFACE_NAME)

        sample_buffer = SampleBuffer(80 * 60)

        while True:
            (msg, address) = skt.recvfrom(MAX_PACKET_LENGTH)
            # TODO: Timestamp should be obtained from socket using SO_TIMESTAMP or similar.
            sample_recv_time = time.time_ns()

            reader = BytesReader(msg)
            _appid = reader.read_u16_be()
            length = reader.read_u16_be()
            _reserved_1 = reader.read_u16_be()
            _reserved_2 = reader.read_u16_be()

            try:
                savpdu = read_sv(reader.sub_reader(length))

                for asdu in savpdu.asdus:
                    sample_buffer.add_sample(sample_recv_time, asdu)

            except RuntimeError as err:
                with open("debug_dump.bin", "wb") as file:
                    file.write(msg)
                err.add_note("packet has been written to debug_dump.bin")
                raise


if __name__ == "__main__":
    main()
