import argparse
import logging
import socket
import struct

import _capng as capng

from .bytes_reader import BytesReader
from .packet import read_sv
from .sample_buffer import SampleBufferManager

INTERFACE_NAME = "lo"
ETHERTYPE_SV = 0x88BA
MAX_PACKET_LENGTH = 1522
SAMPLE_LENGTH = 32
KERNEL_TIMESPEC_LENGTH = 16

# Each of the timestamp socket options has an '*_OLD' and '*_NEW' variant; the '*_OLD' variant
# provides 32-bit timestamps, while '*_NEW' provides 64-bit timestamps. The unsuffixed one is
# then defined as either the '*_OLD' or '*_NEW' one, depending on the platform.
# To ensure that we always receive 64-bit timestamps, we use the '*_NEW' one directly.
# (In any case, Python does not provide these constants in the standard library, so we may as well
# use the same value on all platforms.)
SO_TIMESTAMPNS_NEW = 64


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("-i", "--interface")
    args = parser.parse_args()

    interface = args.interface or "lo"

    logger = logging.getLogger()
    logging.basicConfig(level=logging.DEBUG)

    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as out_skt:
    
        with socket.socket(socket.AF_PACKET, socket.SOCK_DGRAM, ETHERTYPE_SV) as skt:

            # Drop capabilities now that the socket has been opened.
            capng.capng_clear(capng.CAPNG_SELECT_BOTH)
            capng.capng_apply(capng.CAPNG_SELECT_BOTH)

            # Bind the socket to the chosen interface so that we only receive messages on that
            # interface.
            skt.bind((interface, ETHERTYPE_SV))

            # Enable the SO_TIMESTAMPNS_NEW option, which tells the kernel to provide a timestamp
            # with each message. This will be more accurate than reading the time manually.
            skt.setsockopt(socket.SOL_SOCKET, SO_TIMESTAMPNS_NEW, 1)

            logger.info("Successfully bound socket to interface '%s'", interface)

            sample_buffer = SampleBufferManager(80 * 60, out_skt)

            while True:
                # Read the next message from the socket.
                # The timestamps which we requested are sent as control messages (also known as
                # ancillary data). To receive these, we need to use `recvmsg` rather than 'recv'.
                (msg, anc_data, msg_flags, address) = skt.recvmsg(
                    MAX_PACKET_LENGTH, socket.CMSG_SPACE(KERNEL_TIMESPEC_LENGTH)
                )

                # Unpack the timestamp from the control message.
                # The timestamp is provided as two 64 bit integers containing seconds and
                # nanoseconds, respectively.
                (_, _, cmsg_data) = anc_data[0]
                (tv_sec, tv_nsec) = struct.unpack("=qq", cmsg_data)
                sample_recv_time_ns = tv_sec * 1000000000 + tv_nsec

                reader = BytesReader(msg)

                # Read the header of the SV message.
                _appid = reader.read_u16_be()
                length = reader.read_u16_be()
                _reserved_1 = reader.read_u16_be()
                _reserved_2 = reader.read_u16_be()

                try:
                    savpdu = read_sv(reader.sub_reader(length))

                    for asdu in savpdu.asdus:
                        sample_buffer.add_sample(sample_recv_time_ns, asdu)

                except RuntimeError as err:
                    # If an error occurs while decoding the packet, write it to a file to help
                    # debugging.
                    with open("debug_dump.bin", "wb") as file:
                        file.write(msg)
                    err.add_note("packet has been written to debug_dump.bin")
                    raise


if __name__ == "__main__":
    main()
