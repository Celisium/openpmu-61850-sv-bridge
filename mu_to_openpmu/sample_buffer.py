import struct
import time
from io import BufferedWriter
from xml.etree import ElementTree

from .packet import Asdu

NS_PER_SEC = 10**9


class SampleBuffer:
    def __init__(self, sample_rate: int, out_writer: BufferedWriter):
        self._sample_rate = sample_rate
        self._buffer = bytearray(sample_rate * 32)
        self._buffer_start_time_s = 0
        self._out_writer = out_writer

    def add_sample(self, recv_time_ns: int, asdu: Asdu) -> None:
        ns_per_sample = NS_PER_SEC / (self._sample_rate)
        ns_offset = asdu.smp_cnt * ns_per_sample

        recv_time_s = recv_time_ns // NS_PER_SEC

        if ns_offset >= recv_time_ns % NS_PER_SEC:
            recv_time_s -= 1

        if recv_time_s > self._buffer_start_time_s:
            self._flush(recv_time_s)

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

    def _flush(self, recv_time_s: int) -> None:
        buffer_start_time = time.gmtime(recv_time_s)

        root_elem = ElementTree.Element("OpenPMU")

        format_elem = ElementTree.SubElement(root_elem, "format")
        format_elem.text = "Samples"

        date_elem = ElementTree.SubElement(root_elem, "Date")
        date_elem.text = time.strftime("%Y-%m-%d", buffer_start_time)
        time_elem = ElementTree.SubElement(root_elem, "Time")
        time_elem.text = time.strftime("%H:%M:%S", buffer_start_time)

        frame_elem = ElementTree.SubElement(root_elem, "Frame")
        frame_elem.text = "0"

        fs_elem = ElementTree.SubElement(root_elem, "Fs")
        fs_elem.text = "4800"
        n_elem = ElementTree.SubElement(root_elem, "n")
        n_elem.text = "4800"

        bits_elem = ElementTree.SubElement(root_elem, "bits")
        bits_elem.text = "16"

        channels_elem = ElementTree.SubElement(root_elem, "Channels")
        channels_elem.text = "3"

        # voltage_a_elem = bytearray(2 * 80 * 60)
        # voltage_b_elem = bytearray(2 * 80 * 60)
        # voltage_c_elem = bytearray(2 * 80 * 60)

        # for i in range(80 * 60):
        #    (voltage_a, voltage_b, voltage_c) =  = struct.unpack_from("=f"

        self._out_writer.write(self._buffer)

        self._buffer_start_time_s = recv_time_s
        self._buffer = bytearray(len(self._buffer))
