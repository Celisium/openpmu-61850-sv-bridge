use std::{
	ffi::{c_int, c_longlong, c_uint, c_ushort, c_void, CString, OsStr},
	os::{
		fd::{AsRawFd, FromRawFd, OwnedFd},
		unix::ffi::OsStrExt,
	},
};

/// The value of the Ethertype field used IEC 61850-9-2 sampled value messages.
const ETHERTYPE_SV: u16 = 0x88BA;

/// Obtains the index of the network interface with the given name.
fn interface_name_to_index(name: &OsStr) -> std::io::Result<c_uint> {
	// `if_nametoindex` expects a null terminated string.
	let c_name = CString::new(name.as_bytes())
		.map_err(|_| std::io::ErrorKind::InvalidInput)?;

	let index = unsafe { libc::if_nametoindex(c_name.as_ptr()) };
	// `if_nametoindex` returns 0 on error, with the error code in `errno`.
	if index == 0 {
		Err(std::io::Error::last_os_error())
	} else {
		Ok(index)
	}
}

/// A struct providing information about a received Ethernet frame.
#[derive(Debug)]
pub struct RecvInfo {
	/// The length of the frame's payload in bytes.
	pub length: usize,
	pub timestamp_s: i64,
	pub timestamp_ns: u32,
}

#[derive(Debug)]
pub struct EthernetSocket {
	fd: OwnedFd,
}

impl EthernetSocket {

	/// Creates an Ethernet socket which receives Ethernet frames containing sampled value messages.
	/// 
	/// If `interface` is `None`, Ethernet frames will be received from all network interfaces. Otherwise, frames will
	/// only be received on the specified interface.
	pub fn new(interface: Option<&OsStr>) -> std::io::Result<Self> {

		let socket = unsafe {
			libc::socket(
				// The `AF_PACKET` domain is used for Ethernet frames (see the `packet(7)` man page).
				libc::AF_PACKET,
				// For packet sockets, `SOCK_DGRAM` indicates that only the payload should be included.
				// Other information (such as the source MAC address) can be obtained from `recvmsg` if necessary.
				libc::SOCK_DGRAM,
				// Only receive frames with the IEC 61850-9-2 SV Ethertype. `socket` expects this value to be in big
				// endian.
				ETHERTYPE_SV.to_be() as c_int,
			)
		};
		// `socket` returns -1 on error, with the error code in `errno`.
		if socket == -1 {
			return Err(std::io::Error::last_os_error());
		}

		let interface_index = interface
			.map(interface_name_to_index)
			.transpose()?
			.unwrap_or(0);

		let address = libc::sockaddr_ll {
			sll_family: libc::AF_PACKET as c_ushort, // Always `AF_PACKET`.
			sll_protocol: ETHERTYPE_SV.to_be(), // Ethertype can also be specified here, for some reason.
			sll_ifindex: interface_index as c_int, // The interface to receive from. For `bind`, 0 means any interface.
			// Remaining fields are not used for `bind`.
			sll_hatype: 0,
			sll_pkttype: 0,
			sll_halen: 0,
			sll_addr: [0; 8],
		};

		// Bind the socket such that we only receive frames on the specified interface.
		let result = unsafe {
			libc::bind(
				socket,
				&address as *const libc::sockaddr_ll as *const libc::sockaddr,
				size_of::<libc::sockaddr_ll>() as libc::socklen_t,
			)
		};
		// `bind` returns -1 on error, with the error code in `errno`.
		if result == -1 {
			return Err(std::io::Error::last_os_error());
		}

		// Enable the `SO_TIMESTAMPNS_NEW` socket option so that we get a timestamp with each frame received.
		// This timestamp will be more accurate than simply checking the time after receiving a frame, since it does
		// not include the time taken by the kernel to process the frame.
		let optval = 1;
		let result = unsafe {
			libc::setsockopt(
				socket,
				libc::SOL_SOCKET,
				libc::SO_TIMESTAMPNS_NEW,
				&raw const optval as *const c_void,
				size_of::<c_int>() as libc::socklen_t,
			)
		};
		// `setsockopt` returns -1 on error, with the error code in `errno`.
		if result == -1 {
			return Err(std::io::Error::last_os_error());
		}

		Ok(Self {
			fd: unsafe { OwnedFd::from_raw_fd(socket) },
		})
	}

	/// Receives a single Ethernet frame on the socket. The frame's payload will be written to `buf`, while its length
	/// and timestamp are returned in the `RecvInfo` structure.
	/// 
	/// This function will block until a frame is received.
	pub fn recv(&self, buf: &mut [u8]) -> std::io::Result<RecvInfo> {

		// This matches Linux's `__kernel_timespec` type, which uses 64 bit fields even on 32 bit systems.
		#[repr(C)]
		struct KernelTimespec {
			tv_sec: c_longlong,
			tv_nsec: c_longlong,
		}

		// Timestamps are received as control messages (also known as ancillary data), which requires a separate buffer.
		// This buffer must have enough space for both the timestamp and some additional metadata; the total size is
		// calculated using `CMSG_SPACE`.
		const CMSG_BUFFER_LENGTH: usize = unsafe { libc::CMSG_SPACE(size_of::<KernelTimespec>() as u32) } as usize;

		// The control message buffer must have the same alignment as the `cmsghdr` type. A struct is used to control
		// its alignment.
		#[repr(C)]
		struct CMsgBuffer {
			// Since the struct uses the C representation, the first member is guaranteed to be at offset 0, meaning it
			// has the same alignment as the struct.
			buffer: [u8; CMSG_BUFFER_LENGTH],
			// A zero-sized array does not affect the size of the containing struct, but does affect its alignment.
			// Since a struct has the same alignment as its most aligned member, this guarantees that it will have an
			// alignment at least as large as `cmsghdr`.
			_align: [libc::cmsghdr; 0],
		}

		// Create an instance of the struct to hold the buffer.
		let mut cmsg_buffer = CMsgBuffer {
			buffer: [0; CMSG_BUFFER_LENGTH],
			_align: [],
		};

		// The `recvmsg` function is able to write data into several non-contiguous buffers. Since we don't need this
		// feature, we can just specifiy a single buffer.
		let mut msg_iov = libc::iovec {
			iov_base: buf.as_mut_ptr() as *mut c_void,
			iov_len: buf.len(),
		};

		let mut msg = libc::msghdr {
			msg_name: std::ptr::null_mut(), // Can be used if we want to know who sent the frame (for now we don't).
			msg_namelen: 0,
			msg_iov: &raw mut msg_iov,
			msg_iovlen: 1,
			msg_control: cmsg_buffer.buffer.as_mut_ptr() as *mut c_void,
			msg_controllen: cmsg_buffer.buffer.len(),
			msg_flags: 0,
		};

		let length = unsafe { libc::recvmsg(self.fd.as_raw_fd(), &raw mut msg, 0) };
		// `recvmsg` returns -1 on error, with the error code in `errno`.
		if length == -1 {
			return Err(std::io::Error::last_os_error());
		}

		// Iterate through all received control messages to get the one containing the timestamp.
		// This is probably a bit overkill, since the timestamp control message should be the only one present.
		let mut cmsg: *const libc::cmsghdr = unsafe { libc::CMSG_FIRSTHDR(&raw const msg) };
		while !cmsg.is_null() {

			let cmsg_hdr = unsafe { &*cmsg };

			if cmsg_hdr.cmsg_level == libc::SOL_SOCKET && cmsg_hdr.cmsg_type == libc::SO_TIMESTAMPNS_NEW {

				let timestamp_ptr = unsafe { libc::CMSG_DATA(cmsg) } as *const KernelTimespec;
				// The pointer to the control message data is not guaranteed to be aligned.
				let timestamp = unsafe { timestamp_ptr.read_unaligned() };

				return Ok(RecvInfo {
					length: length as usize,
					timestamp_s: timestamp.tv_sec,
					timestamp_ns: timestamp.tv_nsec as u32,
				});
			}

			cmsg = unsafe { libc::CMSG_NXTHDR(&raw const msg, cmsg) };
		}

		unreachable!("did not receive timestamp control message");
	}
}
