use std::{
	ffi::OsString,
	net::{Ipv4Addr, UdpSocket},
};

use clap::Parser;
use mu_rust::{ethernet::EthernetSocket, parse, sample_buffer::SampleBufferManager};

#[derive(Debug, Parser)]
struct CommandLineArgs {
	#[arg(short, long, default_value = "lo")]
	interface: OsString,
	#[arg(short = 'r', long, default_value = "4000")]
	sample_rate: u32,
}

const NOMINAL_FREQUENCY: u32 = 50;

fn main() -> anyhow::Result<()> {
	let args = CommandLineArgs::parse();

	let recv_socket = EthernetSocket::new(Some(&args.interface))?;

	eprintln!("Bound socket to interface '{}'.", &args.interface.to_string_lossy());

	let send_socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;

	let mut buf = [0_u8; 1522]; // The maximum size of an Ethernet frame is 1522 bytes.

	let mut sample_buffer_manager = SampleBufferManager::new(
		args.sample_rate,
		args.sample_rate / (NOMINAL_FREQUENCY * 2),
		send_socket,
	);

	loop {
		let info = recv_socket.recv(&mut buf)?;
		let sv_message = parse(&buf[0..info.length])?;
		for asdu in sv_message.asdus {
			assert!(info.timestamp_s >= 0); // TODO: handle correctly (probably just ignore sample entirely)
			sample_buffer_manager.insert_sample(info.timestamp_s as u64, info.timestamp_ns, asdu);
		}
	}
}
