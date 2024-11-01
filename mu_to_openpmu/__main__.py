import argparse
import logging
import socket
import struct

from .bytes_reader import BytesReader
from .packet import read_sv
from .sample_buffer import SampleBufferManager

INTERFACE_NAME = "lo"
ETHERTYPE_SV = 0x88BA
MAX_PACKET_LENGTH = 1522
SAMPLE_LENGTH = 32

# Python does not provide this definition in the socket module.
SO_TIMESTAMPNS_NEW = 64


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("-i", "--interface")
    args = parser.parse_args()

    interface = args.interface or "lo"

    logger = logging.getLogger()
    logging.basicConfig(level=logging.DEBUG)

    with open("data_out.bin", "wb") as out_file, open("data_out.xml", "wt") as xml_file:
        with socket.socket(socket.AF_PACKET, socket.SOCK_DGRAM, ETHERTYPE_SV) as skt:
            skt.bind((interface, ETHERTYPE_SV))

            skt.setsockopt(socket.SOL_SOCKET, SO_TIMESTAMPNS_NEW, 1)

            logger.info("Successfully bound socket to interface '%s'", interface)

            sample_buffer = SampleBufferManager(80 * 60, out_file, xml_file)

            while True:
                (msg, anc_data, msg_flags, address) = skt.recvmsg(
                    MAX_PACKET_LENGTH, socket.CMSG_SPACE(16)
                )
                (_, _, cmsg_data) = anc_data[0]
                (tv_sec, tv_nsec) = struct.unpack("=qq", cmsg_data)
                sample_recv_time_ns = tv_sec * 1000000000 + tv_nsec

                reader = BytesReader(msg)
                _appid = reader.read_u16_be()
                length = reader.read_u16_be()
                _reserved_1 = reader.read_u16_be()
                _reserved_2 = reader.read_u16_be()

                try:
                    savpdu = read_sv(reader.sub_reader(length))

                    for asdu in savpdu.asdus:
                        sample_buffer.add_sample(sample_recv_time_ns, asdu)

                except RuntimeError as err:
                    with open("debug_dump.bin", "wb") as file:
                        file.write(msg)
                    err.add_note("packet has been written to debug_dump.bin")
                    raise


if __name__ == "__main__":
    main()
