from __future__ import annotations

import base64
import collections
import socket
import struct
import threading
import time
from datetime import datetime, timezone
from xml.etree import ElementTree

from .packet import Asdu, Sample

NS_PER_SEC = 10**9


class SampleBufferChannel:
    def __init__(self, length: int):
        self.buffer = bytearray(length * 4)
        self.max = 0.0

    def add_sample(self, index: int, value: float) -> None:
        struct.pack_into("=f", self.buffer, index * 4, value)
        self.max = max(self.max, abs(value))

    def convert_to_i16(self) -> tuple[bytearray, float]:
        length = len(self.buffer) // 4
        converted_buffer = bytearray(length * 2)

        if self.max == 0:
            return (converted_buffer, 0)

        for i in range(length):
            (original,) = struct.unpack_from("=f", self.buffer, i * 4)
            converted = int(original / self.max * 32767)
            struct.pack_into(">h", converted_buffer, i * 2, converted)

        return (converted_buffer, self.max)


class SampleBuffer:
    def __init__(
        self,
        sample_rate: int,
        start_time_s: int,
        sample_offset: int,
        length: int,
    ):
        self._channels = [SampleBufferChannel(length) for _ in range(8)]
        self._sample_rate = sample_rate
        self.start_time_s = start_time_s
        self.sample_offset = sample_offset
        self.length = length

    def add_sample(self, smp_cnt: int, sample: Sample) -> None:
        index = smp_cnt - self.sample_offset
        self._channels[0].add_sample(index, sample.current_a)
        self._channels[1].add_sample(index, sample.current_b)
        self._channels[2].add_sample(index, sample.current_c)
        self._channels[3].add_sample(index, sample.current_n)
        self._channels[4].add_sample(index, sample.voltage_a)
        self._channels[5].add_sample(index, sample.voltage_b)
        self._channels[6].add_sample(index, sample.voltage_c)
        self._channels[7].add_sample(index, sample.voltage_n)

    def flush(self, out_skt: socket.socket) -> None:
        if self.start_time_s == 0:
            return

        start_time_s = self.start_time_s + (self.sample_offset / self._sample_rate)

        start_time_utc = datetime.fromtimestamp(start_time_s, timezone.utc)

        root_elem = ElementTree.Element("OpenPMU")

        format_elem = ElementTree.SubElement(root_elem, "Format")
        format_elem.text = "Samples"

        date_elem = ElementTree.SubElement(root_elem, "Date")
        date_elem.text = start_time_utc.strftime("%Y-%m-%d")
        time_elem = ElementTree.SubElement(root_elem, "Time")
        time_elem.text = start_time_utc.strftime("%H:%M:%S.%f")

        frame_elem = ElementTree.SubElement(root_elem, "Frame")
        # TODO: Support nominal frequencies other than 60 Hz.
        frame_elem.text = str(int(self.sample_offset / self._sample_rate * 120))

        fs_elem = ElementTree.SubElement(root_elem, "Fs")
        fs_elem.text = str(self._sample_rate)
        n_elem = ElementTree.SubElement(root_elem, "n")
        n_elem.text = str(self.length)

        bits_elem = ElementTree.SubElement(root_elem, "bits")
        bits_elem.text = "16"

        channels_elem = ElementTree.SubElement(root_elem, "Channels")
        channels_elem.text = "6"

        (current_a_data, current_a_max) = self._channels[1].convert_to_i16()
        (current_b_data, current_b_max) = self._channels[2].convert_to_i16()
        (current_c_data, current_c_max) = self._channels[3].convert_to_i16()
        (voltage_a_data, voltage_a_max) = self._channels[4].convert_to_i16()
        (voltage_b_data, voltage_b_max) = self._channels[5].convert_to_i16()
        (voltage_c_data, voltage_c_max) = self._channels[6].convert_to_i16()

        def build_channel(
            index: int, name: str, type: str, phase: str, range: float, data: bytes
        ) -> ElementTree.Element:
            channel_elem = ElementTree.SubElement(root_elem, "Channel_{}".format(index))
            name_elem = ElementTree.SubElement(channel_elem, "Name")
            name_elem.text = name
            type_elem = ElementTree.SubElement(channel_elem, "Type")
            type_elem.text = type
            phase_elem = ElementTree.SubElement(channel_elem, "Phase")
            phase_elem.text = phase
            range_elem = ElementTree.SubElement(channel_elem, "Range")
            range_elem.text = str(range)
            payload_elem = ElementTree.SubElement(channel_elem, "Payload")
            payload_elem.text = str(base64.b64encode(data), "ascii")
            return channel_elem

        build_channel(0, "Belfast_Va", "V", "a", voltage_a_max, voltage_a_data)
        build_channel(1, "Belfast_Vb", "V", "b", voltage_b_max, voltage_b_data)
        build_channel(2, "Belfast_Vc", "V", "c", voltage_c_max, voltage_c_data)
        build_channel(3, "Belfast_Ia", "I", "a", current_a_max, current_a_data)
        build_channel(4, "Belfast_Ib", "I", "b", current_b_max, current_b_data)
        build_channel(5, "Belfast_Ic", "I", "c", current_c_max, current_c_data)

        ElementTree.indent(root_elem)
        out_skt.sendto(
            bytes(ElementTree.tostring(root_elem, "unicode"), "utf-8"), ("127.0.0.1", 48001)
        )

    def is_sample_within_timespan(self, seconds: int, count: int) -> bool:
        buffer_start_time = self.start_time_s * self._sample_rate + self.sample_offset
        buffer_end_time = buffer_start_time + self.length

        sample_time = seconds * self._sample_rate + count

        return buffer_start_time <= sample_time < buffer_end_time

    def is_sample_after_timespan(self, seconds: int, count: int) -> bool:
        buffer_end_time_in_sample_counts = (
            self.start_time_s * self._sample_rate + self.sample_offset + self.length
        )

        sample_time_in_sample_counts = seconds * self._sample_rate + count

        return sample_time_in_sample_counts >= buffer_end_time_in_sample_counts

    def get_send_time(self) -> float:
        return self.start_time_s + (self.sample_offset + self.length) / self._sample_rate + 0.005


class SampleBufferManager:
    """
    Manager class for sample buffers.

    This class is responsible for keeping track of the current and previous sample buffers. When a
    new sample is recevied (using the `add_sample` method), it will determine whether it can go
    into an existing sample buffer, and if so it will store it in that buffer.

    If it falls outside the timespans of the current and previous buffers, a new one will be
    created. This is done by flushing the previous buffer and replacing it with the 'old' current
    buffer, which is itself replaced by a newly created buffer.
    """

    def __init__(self, sample_rate: int, buffer_length: int, out_socket: socket.socket):
        self._sample_rate = sample_rate
        self._buffer_length = buffer_length

        self._buffer_queue: collections.deque[SampleBuffer] = collections.deque()
        self._buffer_queue_lock = threading.Lock()
        self._buffer_queue_cond = threading.Condition(self._buffer_queue_lock)

        self._socket = out_socket

        self._sender_thread = threading.Thread(
            target=self._sender_thread_fn, daemon=True
        )
        self._sender_thread.start()

    def add_sample(self, recv_time_ns: int, asdu: Asdu) -> None:
        """Add a sample to a buffer, flushing the previous buffer if necessary."""
        ns_per_sample = NS_PER_SEC / (self._sample_rate)
        ns_offset = asdu.smp_cnt * ns_per_sample

        recv_time_s = recv_time_ns // NS_PER_SEC

        if ns_offset >= recv_time_ns % NS_PER_SEC:
            recv_time_s -= 1

        with self._buffer_queue_lock:
            if len(self._buffer_queue) == 0 or self._buffer_queue[-1].is_sample_after_timespan(
                recv_time_s, asdu.smp_cnt
            ):
                new_buffer = SampleBuffer(
                    self._sample_rate,
                    recv_time_s,
                    asdu.smp_cnt // self._buffer_length * self._buffer_length,
                    self._buffer_length,
                )
                new_buffer.add_sample(asdu.smp_cnt, asdu.sample)
                self._buffer_queue.append(new_buffer)
                self._buffer_queue_cond.notify()

            else:
                buffer = next(
                    filter(
                        lambda buffer: buffer.is_sample_within_timespan(recv_time_s, asdu.smp_cnt),
                        reversed(self._buffer_queue),
                    )
                )
                buffer.add_sample(asdu.smp_cnt, asdu.sample)

    def _sender_thread_fn(self) -> None:
        while True:
            with self._buffer_queue_lock:
                self._buffer_queue_cond.wait_for(lambda: len(self._buffer_queue) > 0)
                sleep_time = self._buffer_queue[0].get_send_time() - time.time() + 1

            #print(f"sleeping for {sleep_time} seconds")
            if sleep_time > 0:
                time.sleep(sleep_time)

            with self._buffer_queue_lock:
                buffer = self._buffer_queue.popleft()

            buffer.flush(self._socket)
