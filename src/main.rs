use std::{
	ffi::OsStr,
	net::{Ipv4Addr, UdpSocket},
	path::PathBuf,
};

use clap::Parser;
use mu_rust::{
	config::Configuration,
	ethernet::EthernetSocket,
	parse,
	sample_buffer::{SampleBufferQueue, sender_thread_fn},
};

#[derive(Debug, Parser)]
struct CommandLineArgs {
	#[arg(short, long)]
	config: PathBuf,
}

fn main() -> anyhow::Result<()> {
	let env = env_logger::Env::default().default_filter_or("info");
	env_logger::init_from_env(env);

	let args = CommandLineArgs::parse();

	let config_file_str = match std::fs::read_to_string(&args.config) {
		Ok(s) => s,
		Err(err) => {
			log::error!("Unable to read configuration file '{}': {err}", args.config.display());
			std::process::exit(1);
		},
	};

	let configuration = match toml::from_str::<Configuration>(&config_file_str) {
		Ok(c) => c,
		Err(err) => {
			log::error!("Unable to read configuration file '{}': {err}", args.config.display());
			std::process::exit(1);
		},
	};

	let recv_socket = EthernetSocket::new(Some(OsStr::new(&configuration.interface)))?;

	log::info!("Bound socket to interface '{}'.", &configuration.interface);

	let mut buf = [0_u8; 1522]; // The maximum size of an Ethernet frame is 1522 bytes.

	let buffer_length = configuration.sample_rate / (configuration.nominal_frequency * 2);

	let send_socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;

	let sample_buffer_queue = SampleBufferQueue::new();

	log::info!("Datagrams will be sent to {}.", &configuration.destination);

	std::thread::scope(|scope| {
		let _sender_thread = scope.spawn(|| sender_thread_fn(&sample_buffer_queue, send_socket, configuration.destination, &configuration.channels));
		loop {
			let info = recv_socket.recv(&mut buf)?;
			let sv_message = parse(&buf[0..info.length])?;
			for asdu in sv_message.asdus {
				assert!(info.timestamp_s >= 0); // TODO: handle correctly (probably just ignore sample entirely)
				sample_buffer_queue.insert_sample(
					info.timestamp_s as u64,
					info.timestamp_ns,
					configuration.sample_rate,
					buffer_length,
					asdu,
				);
			}
		}
	})
}
