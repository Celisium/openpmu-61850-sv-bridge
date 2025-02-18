use std::{
	ffi::OsString,
	net::{Ipv4Addr, SocketAddr, UdpSocket},
};

use clap::Parser;
use mu_rust::{
	ethernet::EthernetSocket,
	parse,
	sample_buffer::{sender_thread_fn, SampleBufferQueue},
};

#[derive(Debug, Parser)]
struct CommandLineArgs {
	#[arg(short, long, default_value = "lo")]
	interface: OsString,
	#[arg(short = 'r', long, default_value = "4000")]
	sample_rate: u32,
	#[arg(short = 'f', long)]
	nominal_freq: Option<u32>,
	#[arg(short = 'd', long, default_value = "127.0.0.1:48001")]
	dest: SocketAddr,
}

fn main() -> anyhow::Result<()> {
	env_logger::init();

	let args = CommandLineArgs::parse();

	let nominal_freq = match (args.nominal_freq, args.sample_rate) {
		(Some(nominal_freq), _) => nominal_freq,
		(None, 4000 | 12800) => 50,
		(None, 4800 | 15360) => 60,
		_ => {
			log::error!("Unable to guess nominal frequency from the sample rate ({} Hz).", args.sample_rate);
			log::error!("The nominal frequency can be specified using the `--nominal-freq` option.");
			std::process::exit(1);
		},
	};

	if args.nominal_freq.is_none() {
		log::warn!("Nominal frequency was not specified; assuming {nominal_freq} Hz based on sample rate.");
	}

	let recv_socket = EthernetSocket::new(Some(&args.interface))?;

	log::info!("Bound socket to interface '{}'.", &args.interface.to_string_lossy());

	let mut buf = [0_u8; 1522]; // The maximum size of an Ethernet frame is 1522 bytes.

	let buffer_length = args.sample_rate / (nominal_freq * 2);

	let send_socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;

	let sample_buffer_queue = SampleBufferQueue::new();

	log::info!("Datagrams will be sent to {}.", &args.dest);

	std::thread::scope(|scope| {
		let _sender_thread = scope.spawn(|| sender_thread_fn(&sample_buffer_queue, send_socket, args.dest));
		loop {
			let info = recv_socket.recv(&mut buf)?;
			let sv_message = parse(&buf[0..info.length])?;
			for asdu in sv_message.asdus {
				assert!(info.timestamp_s >= 0); // TODO: handle correctly (probably just ignore sample entirely)
				sample_buffer_queue.insert_sample(
					info.timestamp_s as u64,
					info.timestamp_ns,
					args.sample_rate,
					buffer_length,
					asdu,
				);
			}
		}
	})
}
