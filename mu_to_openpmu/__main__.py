import logging
import socket
import time

from .bytes_reader import BytesReader
from .packet import read_sv
from .sample_buffer import SampleBufferManager

INTERFACE_NAME = "lo"
ETHERTYPE_SV = 0x88BA
MAX_PACKET_LENGTH = 1522
SAMPLE_LENGTH = 32


def main() -> None:
    logger = logging.getLogger()
    logging.basicConfig(level=logging.DEBUG)

    with open("data_out.bin", "wb") as out_file, open("data_out.xml", "wt") as xml_file:
        with socket.socket(socket.AF_PACKET, socket.SOCK_DGRAM, ETHERTYPE_SV) as skt:
            skt.bind((INTERFACE_NAME, ETHERTYPE_SV))

            logger.info("Successfully bound socket to interface '%s'", INTERFACE_NAME)

            sample_buffer = SampleBufferManager(80 * 60, out_file, xml_file)

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
