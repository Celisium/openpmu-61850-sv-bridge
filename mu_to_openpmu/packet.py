import struct
from dataclasses import dataclass

from . import asn1
from .bytes_reader import BytesReader


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


def read_utctime(reader: BytesReader) -> UtcTime:
    contents = reader.read_bytes(8)

    # ssssssssssssssssssssssssssssssss ffffffffffffffffffffffff qqqqqqqq
    (secs, frac_qual) = struct.unpack(">II", contents)
    secs += (frac_qual >> 8) / (1 << 24)

    quality = TimeQuality(
        (frac_qual & 0b10000000) != 0,
        (frac_qual & 0b01000000) != 0,
        (frac_qual & 0b00100000) != 0,
        frac_qual & 0b00011111,  # TODO: Use separate values for invalid/unspecified
    )

    return UtcTime(secs, quality)


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


def read_sample(reader: BytesReader) -> Sample:
    return Sample(reader.read_bytes(64))


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
    tag = asn1.read_tag(reader)
    if tag.num == 0 and tag.is_context_specific():
        length = asn1.read_length(reader)
        svid = asn1.read_visiblestring(reader.sub_reader(length), tag, length)
        reader.skip(length)
    else:
        raise RuntimeError("Expected context-specific tag with tag number 0")

    # datset [1] IMPLICIT VisibleString OPTIONAL
    tag = asn1.read_tag(reader)
    if tag.num == 1 and tag.is_context_specific():
        length = asn1.read_length(reader)
        datset = asn1.read_visiblestring(reader.sub_reader(length), tag, length)
        reader.skip(length)
        tag = asn1.read_tag(reader)
    else:
        datset = None

    # smpCnt [2] IMPLICIT OCTET STRING (SIZE(2))
    if tag.num == 2 and tag.is_context_specific():
        length = asn1.read_length(reader)
        if length != 2:
            raise RuntimeError("Expected octet string with length 2")
        smp_cnt_bytes = asn1.read_octet_string(reader.sub_reader(length), tag, length)
        reader.skip(length)
        (smp_cnt,) = struct.unpack(">H", smp_cnt_bytes)
    else:
        raise RuntimeError("Expected context-specific tag with tag number 2")

    # confRev [3] IMPLICIT OCTET STRING (SIZE(4))
    tag = asn1.read_tag(reader)
    if tag.num == 3 and tag.is_context_specific():
        length = asn1.read_length(reader)
        if length != 4:
            raise RuntimeError("Expected octet string with length 4")
        conf_rev_bytes = asn1.read_octet_string(reader.sub_reader(length), tag, length)
        reader.skip(length)
        (conf_rev,) = struct.unpack(">I", conf_rev_bytes)
    else:
        raise RuntimeError("Expected context-specific tag with tag number 3")

    # refrTm [4] IMPLICIT UtcTime OPTIONAL
    # (This is not the universal ASN.1 UTCTime type, but the IEC 61850 UtcTime type.)
    tag = asn1.read_tag(reader)
    if tag.num == 4 and tag.is_context_specific():
        length = asn1.read_length(reader)
        refr_tm = read_utctime(reader.sub_reader(length))
        reader.skip(length)
        tag = asn1.read_tag(reader)
    else:
        refr_tm = None

    # smpSynch [5] IMPLICIT OCTET STRING (SIZE(1))
    if tag.num == 5 and tag.is_context_specific():
        length = asn1.read_length(reader)
        if length != 1:
            raise RuntimeError("Expected octet string with length 1")
        smp_synch = reader.read_u8()
    else:
        raise RuntimeError("Expected context-specific tag with tag number 5")

    # smpRate [6] IMPLICIT OCTET STRING (SIZE(2)) OPTIONAL
    tag = asn1.read_tag(reader)
    if tag.num == 6 and tag.is_context_specific():
        length = asn1.read_length(reader)
        if length != 2:
            raise RuntimeError("Expected octet string with length 2")
        smp_rate_bytes = asn1.read_octet_string(reader.sub_reader(length), tag, length)
        reader.skip(length)
        (smp_rate,) = struct.unpack(">H", smp_rate_bytes)
        tag = asn1.read_tag(reader)
    else:
        smp_rate = None

    # sample [7] IMPLICIT OCTET STRING (SIZE(n))
    if tag.num == 7 and tag.is_context_specific():
        length = asn1.read_length(reader)
        sample = read_sample(reader)
    else:
        raise RuntimeError("Expected context-specific tag with tag number 7")

    # TODO: Two more optional values

    return Asdu(svid, datset, smp_cnt, conf_rev, refr_tm, smp_synch, smp_rate, sample)


@dataclass
class SavPdu:
    asdus: list[Asdu]


def read_savpdu(reader: BytesReader) -> SavPdu:
    # noASDU [0] IMPLICIT INTEGER (1..65535)
    tag = asn1.read_tag(reader)
    if tag.num == 0 and tag.is_context_specific():
        length = asn1.read_length(reader)
        no_asdu = asn1.read_integer(reader.sub_reader(length), length)
        reader.skip(length)
        if not 1 <= no_asdu < 65536:
            raise RuntimeError("noASDU out of range")
    else:
        raise RuntimeError("Expected context-specific tag with tag number 0")

    # security [1] ANY OPTIONAL
    tag = asn1.read_tag(reader)
    if tag.num == 1 and tag.is_context_specific():
        length = asn1.read_length(reader)
        reader.skip(length)
        tag = asn1.read_tag(reader)

    # asdu [2] IMPLICIT SEQUENCE OF ASDU
    if tag.num == 2 and tag.is_context_specific():
        length = asn1.read_length(reader)
        inner_reader = reader.sub_reader(length)

        asdus = []
        for _ in range(no_asdu):
            tag = asn1.read_tag(inner_reader)
            if tag.num == 16 and tag.is_universal():
                length = asn1.read_length(inner_reader)
                asdu = read_asdu(inner_reader.sub_reader(length))
                asdus.append(asdu)
                inner_reader.skip(length)
            else:
                raise RuntimeError("Expected universal tag with tag number 16")

    else:
        raise RuntimeError("Expected context-specific tag with tag number 2")

    return SavPdu(asdus)


def read_sv(reader: BytesReader) -> SavPdu:
    tag = asn1.read_tag(reader)
    if tag.num == 0 and tag.is_application():
        length = asn1.read_length(reader)
        savpdu = read_savpdu(reader.sub_reader(length))
        reader.skip(length)
    else:
        raise RuntimeError("Unexpected tag number/class")
    return savpdu
