mod ber;
mod bytes;
mod ethernet;
mod sample_buffer;

use std::{
	ffi::OsString,
	net::{Ipv4Addr, UdpSocket},
};

use clap::Parser;

use ber::{DecodeError, Encoding, Tag};
use bytes::BytesReader;
use ethernet::EthernetSocket;
use sample_buffer::SampleBufferManager;

fn read_iec61850_int8u(reader: &mut BytesReader<'_>, encoding: Encoding) -> Result<u8, DecodeError> {
	if let &[b_0] = ber::read_octet_string(reader, encoding)? {
		Ok(b_0)
	} else {
		// TODO: Specific error type.
		Err(DecodeError::InvalidIntegerEncoding)
	}
}

fn read_iec61850_int16u(reader: &mut BytesReader<'_>, encoding: Encoding) -> Result<u16, DecodeError> {
	if let &[b_0, b_1] = ber::read_octet_string(reader, encoding)? {
		Ok(u16::from_be_bytes([b_0, b_1]))
	} else {
		// TODO: Specific error type.
		Err(DecodeError::InvalidIntegerEncoding)
	}
}

fn read_iec61850_int32u(reader: &mut BytesReader<'_>, encoding: Encoding) -> Result<u32, DecodeError> {
	if let &[b_0, b_1, b_2, b_3] = ber::read_octet_string(reader, encoding)? {
		Ok(u32::from_be_bytes([b_0, b_1, b_2, b_3]))
	} else {
		// TODO: Specific error type.
		Err(DecodeError::InvalidIntegerEncoding)
	}
}

fn read_iec61850_utctime(reader: &mut BytesReader<'_>, encoding: Encoding) -> Result<u64, DecodeError> {
	if let &[b_0, b_1, b_2, b_3, b_4, b_5, b_6, b_7] = ber::read_octet_string(reader, encoding)? {
		Ok(u64::from_be_bytes([b_0, b_1, b_2, b_3, b_4, b_5, b_6, b_7]))
	} else {
		// TODO: Specific error type.
		Err(DecodeError::InvalidIntegerEncoding)
	}
}

#[derive(Debug, Clone, Default)]
struct Sample {
	current_a: f32,
	current_b: f32,
	current_c: f32,
	current_n: f32,
	voltage_a: f32,
	voltage_b: f32,
	voltage_c: f32,
	voltage_n: f32,
}

impl Sample {
	fn read(reader: &mut BytesReader<'_>, encoding: Encoding) -> Result<Self, DecodeError> {
		let bytes = ber::read_octet_string(reader, encoding)?;
		if bytes.len() != 64 {
			// TODO: Specific error type.
			return Err(DecodeError::InvalidIntegerEncoding);
		}

		let mut values_iter = bytes
			.chunks_exact(8)
			.map(|chunk| i32::from_be_bytes(chunk[0..4].try_into().unwrap()) as f64);

		let current_scale = 0.001;
		let voltage_scale = 0.01;

		Ok(Self {
			current_a: (values_iter.next().unwrap() * current_scale) as f32,
			current_b: (values_iter.next().unwrap() * current_scale) as f32,
			current_c: (values_iter.next().unwrap() * current_scale) as f32,
			current_n: (values_iter.next().unwrap() * current_scale) as f32,
			voltage_a: (values_iter.next().unwrap() * voltage_scale) as f32,
			voltage_b: (values_iter.next().unwrap() * voltage_scale) as f32,
			voltage_c: (values_iter.next().unwrap() * voltage_scale) as f32,
			voltage_n: (values_iter.next().unwrap() * voltage_scale) as f32,
		})
	}
}

#[derive(Debug, Clone)]
struct Asdu {
	svid: String,
	datset: Option<String>,
	smp_cnt: u16,
	conf_rev: u32,
	refr_tm: Option<u64>,
	smp_synch: u8,
	smp_rate: Option<u16>,
	sample: Sample,
	smp_mod: Option<u16>,
}

fn read_asdu(reader: &mut BytesReader<'_>) -> Result<Asdu, DecodeError> {
	// svID [0] IMPLICIT VisibleString
	let svid = ber::read_required_identifier(reader, Tag::ContextSpecific(0))
		.and_then(|encoding| ber::read_visiblestring(reader, encoding))?;

	// datset [1] IMPLICIT VisibleString OPTIONAL
	let datset = ber::read_optional_identifier(reader, Tag::ContextSpecific(1))?
		.map(|encoding| ber::read_visiblestring(reader, encoding))
		.transpose()?;

	// smpCnt [2] IMPLICIT OCTET STRING (SIZE(2))
	let smp_cnt = ber::read_required_identifier(reader, Tag::ContextSpecific(2))
		.and_then(|encoding| read_iec61850_int16u(reader, encoding))?;

	// confRev [3] IMPLICIT OCTET STRING (SIZE(4))
	let conf_rev = ber::read_required_identifier(reader, Tag::ContextSpecific(3))
		.and_then(|encoding| read_iec61850_int32u(reader, encoding))?;

	// refrTm [4] IMPLICIT UtcTime OPTIONAL
	// (This is not the universal ASN.1 UTCTime type, but the IEC 61850 UtcTime type)
	let refr_tm = ber::read_optional_identifier(reader, Tag::ContextSpecific(4))?
		.map(|encoding| read_iec61850_utctime(reader, encoding))
		.transpose()?;

	// smpSynch [5] IMPLICIT OCTET STRING (SIZE(1))
	let smp_synch = ber::read_required_identifier(reader, Tag::ContextSpecific(5))
		.and_then(|encoding| read_iec61850_int8u(reader, encoding))?;

	// smpRate [6] IMPLICIT OCTET STRING (SIZE(2)) OPTIONAL
	let smp_rate = ber::read_optional_identifier(reader, Tag::ContextSpecific(6))?
		.map(|encoding| read_iec61850_int16u(reader, encoding))
		.transpose()?;

	// sample [7] IMPLICIT OCTET STRING (SIZE(n))
	let sample = ber::read_required_identifier(reader, Tag::ContextSpecific(7))
		.and_then(|encoding| Sample::read(reader, encoding))?;

	// smpMod [8] IMPLICIT OCTET STRING (SIZE(2)) OPTIONAL
	let smp_mod = ber::read_optional_identifier(reader, Tag::ContextSpecific(8))?
		.map(|encoding| read_iec61850_int16u(reader, encoding))
		.transpose()?;

	// TODO: gmIdentity [9] IMPLICIT OCTET STRING (SIZE(8)) OPTIONAL

	Ok(Asdu {
		svid: svid.into(),
		datset: datset.map(Into::into),
		smp_cnt,
		conf_rev,
		refr_tm,
		smp_synch,
		sample,
		smp_rate,
		smp_mod,
	})
}

fn read_savpdu(reader: &mut BytesReader<'_>) -> Result<Vec<Asdu>, DecodeError> {
	// noASDU [0] IMPLICIT INTEGER (1..65535)
	let encoding = ber::read_required_identifier(reader, Tag::ContextSpecific(0))?;
	let no_asdu = ber::read_integer_as_u16(reader, encoding)?;

	if no_asdu == 0 {
		return Err(DecodeError::TagOutOfRange);
	}

	// security [1] ANY OPTIONAL
	if ber::read_optional_identifier(reader, Tag::ContextSpecific(1))?.is_some() {
		let length = ber::read_length(reader)?;
		reader.skip(length)?;
	}

	// asdu [2] IMPLICIT SEQUENCE OF ASDU
	let _ = ber::read_required_identifier(reader, Tag::ContextSpecific(2))?;
	let length = ber::read_length(reader)?;
	let mut inner_reader = reader.take_sub_reader(length)?;

	(0..no_asdu)
		.map(|_| {
			let _ = ber::read_required_identifier(&mut inner_reader, Tag::Universal(16))?;
			let length = ber::read_length(&mut inner_reader)?;
			read_asdu(&mut inner_reader.take_sub_reader(length)?)
		})
		.collect::<Result<Vec<_>, _>>()
}

#[derive(Debug, Clone)]
struct SvMessage {
	appid: u16,
	asdus: Vec<Asdu>,
}

fn parse(bytes: &[u8]) -> anyhow::Result<SvMessage> {
	let mut reader = BytesReader::new(bytes);

	let appid = reader.read_u16_be()?;
	let length = reader.read_u16_be()? as usize;
	let _reserved_1 = reader.read_u16_be()?;
	let _reserved_2 = reader.read_u16_be()?;

	if length < 8 {
		return Err(DecodeError::LengthOutOfRange.into());
	}

	reader.limit(length - 8)?;

	let _ = ber::read_required_identifier(&mut reader, Tag::Application(0))?;
	let length = ber::read_length(&mut reader)?;
	reader.limit(length)?;
	let asdus = read_savpdu(&mut reader)?;

	Ok(SvMessage { appid, asdus })
}

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

	let mut sample_buffer_manager =
		SampleBufferManager::new(args.sample_rate, args.sample_rate / (NOMINAL_FREQUENCY * 2), send_socket);

	loop {
		let info = recv_socket.recv(&mut buf)?;
		let sv_message = parse(&buf[0..info.length])?;
		for asdu in sv_message.asdus {
			assert!(info.timestamp_s >= 0); // TODO: handle correctly (probably just ignore sample entirely)
			sample_buffer_manager.insert_sample(info.timestamp_s as u64, info.timestamp_ns, asdu);
		}
	}
}
