use std::{
	ffi::{c_int, c_longlong, c_uint, c_ushort, c_void, CString, OsStr},
	os::{
		fd::{AsRawFd, FromRawFd, OwnedFd},
		unix::ffi::OsStrExt
	}
};

const ETHERTYPE_SV: u16 = 0x88BA;

fn interface_name_to_index(name: &OsStr) -> std::io::Result<c_uint> {
	let c_name = CString::new(name.as_bytes())
		.map_err(|_| std::io::ErrorKind::InvalidInput)?;

	let index = unsafe { libc::if_nametoindex(c_name.as_ptr()) };
	if index == 0 {
		Err(std::io::Error::last_os_error())
	} else {
		Ok(index)
	}
}

#[derive(Debug)]
pub struct RecvInfo {
	pub length: usize,
	pub timestamp_s: i64,
	pub timestamp_ns: u32
}

#[derive(Debug)]
pub struct EthernetSocket {
	fd: OwnedFd
}

impl EthernetSocket {

	pub fn new(interface: Option<&OsStr>) -> std::io::Result<Self> {

		let socket = unsafe { libc::socket(libc::AF_PACKET, libc::SOCK_DGRAM, ETHERTYPE_SV.to_be() as c_int) };
		if socket == -1 {
			return Err(std::io::Error::last_os_error());
		}

		let interface_index = interface
			.map(interface_name_to_index)
			.transpose()?
			.unwrap_or(0);

		let address = libc::sockaddr_ll {
			sll_family: libc::AF_PACKET as c_ushort,
			sll_protocol: ETHERTYPE_SV.to_be(),
			sll_ifindex: interface_index as c_int,
			sll_hatype: 0,
			sll_pkttype: 0,
			sll_halen: 0,
			sll_addr: [0; 8],
		};

		let result = unsafe {
			libc::bind(
				socket,
				&address as *const libc::sockaddr_ll as *const libc::sockaddr,
				size_of::<libc::sockaddr_ll>() as libc::socklen_t
			)
		};
		if result == -1 {
			return Err(std::io::Error::last_os_error());
		}

		let optval = 1;
		let result = unsafe {
			libc::setsockopt(
				socket,
				libc::SOL_SOCKET,
				libc::SO_TIMESTAMPNS_NEW,
				&optval as *const c_int as *const c_void,
				size_of::<c_int>() as libc::socklen_t
			)
		};
		if result == -1 {
			return Err(std::io::Error::last_os_error());
		}

		Ok(Self {
			fd: unsafe { OwnedFd::from_raw_fd(socket) }
		})
	}

	pub fn recv(&self, buf: &mut [u8]) -> std::io::Result<RecvInfo> {

		const CMSG_BUFFER_LENGTH: u32 = unsafe { libc::CMSG_SPACE(size_of::<KernelTimespec>() as u32) };

		// This matches Linux's __kernel_timespec type, which uses 64 bit fields even on 32 bit systems.
		#[repr(C)]
		struct KernelTimespec {
			tv_sec: c_longlong,
			tv_nsec: c_longlong,
		}

		// Wrapper struct for the control message buffer, to ensure that it has the correct alignment.
		struct CMsgBuffer {
			buffer: [u8; CMSG_BUFFER_LENGTH as usize],
			_alignment: [libc::cmsghdr; 0]
		}

		let mut msg_iov = libc::iovec {
			iov_base: buf.as_mut_ptr() as *mut c_void,
			iov_len: buf.len(),
		};

		let mut cmsg_buffer = CMsgBuffer {
			buffer: [0; CMSG_BUFFER_LENGTH as usize],
			_alignment: []
		};

		let mut msg = libc::msghdr {
			msg_name: std::ptr::null_mut(),
			msg_namelen: 0,
			msg_iov: &mut msg_iov,
			msg_iovlen: 1,
			msg_control: cmsg_buffer.buffer.as_mut_ptr() as *mut c_void,
			msg_controllen: cmsg_buffer.buffer.len(),
			msg_flags: 0,
		};

		let length = unsafe { libc::recvmsg(self.fd.as_raw_fd(), &mut msg, 0) };
		if length == -1 {
			return Err(std::io::Error::last_os_error());
		}

		let mut cmsg: *const libc::cmsghdr = unsafe { libc::CMSG_FIRSTHDR(&msg) };
		while !cmsg.is_null() {

			let cmsg_hdr = unsafe { &*cmsg };

			// For some reason there is no SCM_TIMESTAMPNS_NEW so I used the SO_TIMESTAMPNS_NEW instead.
			if cmsg_hdr.cmsg_level == libc::SOL_SOCKET && cmsg_hdr.cmsg_type == libc::SO_TIMESTAMPNS_NEW {

				let timestamp_ptr = unsafe { libc::CMSG_DATA(cmsg) } as *const KernelTimespec;
				let timestamp = unsafe { timestamp_ptr.read_unaligned() };

				return Ok(RecvInfo {
					length: length as usize,
					timestamp_s: timestamp.tv_sec,
					timestamp_ns: timestamp.tv_nsec as u32
				});
			}

			cmsg = unsafe { libc::CMSG_NXTHDR(&msg, cmsg) };
		}

		//log::error!("Received a packet without a timestamp control message.");

		Err(std::io::ErrorKind::InvalidData.into())
	}
}
