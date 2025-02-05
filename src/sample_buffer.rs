use std::{
	collections::VecDeque,
	fmt::Write,
	net::UdpSocket,
	sync::{Arc, Condvar, Mutex},
	thread::JoinHandle,
	time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::Engine;
use time::OffsetDateTime;

use crate::{Asdu, Sample, NOMINAL_FREQUENCY};

// TODO: Terminology is somewhat inconsistent e.g. 'buffer' refers to both the buffer field in SampleBufferChannel and
//       the SampleBuffer struct (which contains several channels).

/// A timestamp represented as the number of sample periods since 1 January 1970 00:00:00 UTC.
/// This value is only meaningful with a known sample rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SampleTime(u64);

impl SampleTime {
	pub fn from_seconds_and_samples(seconds: u64, samples: u32, sample_rate: u32) -> Self {
		Self(seconds * sample_rate as u64 + samples as u64)
	}
	pub fn seconds(self, sample_rate: u32) -> u64 {
		self.0 / sample_rate as u64
	}
	pub fn samples(self, sample_rate: u32) -> u32 {
		(self.0 % sample_rate as u64) as u32
	}
	pub fn add_samples(self, samples: u32) -> Self {
		Self(self.0 + samples as u64)
	}
	pub fn to_secs_f64(self, sample_rate: u32) -> f64 {
		self.0 as f64 / sample_rate as f64
	}
}

/// A struct containing sample data for a single channel in a sample buffer. The `SampleBuffer` struct contains one
/// `SampleBufferChannel` for each voltage or current channel.
/// 
/// This struct also keeps track of the largest absolute value currently stored in the buffer. This avoids the need to
/// search through the entire buffer later.
#[derive(Debug)]
pub struct SampleBufferChannel {
	/// Array of sample data for this channel.
	buffer: Box<[f32]>,
	/// The largest absolute value stored in this channel buffer.
	max: f32,
}

impl SampleBufferChannel {
	/// Creates a new sample buffer channel containing `length` samples, with each sample initialised to zero.
	pub fn new(length: usize) -> Self {
		let buffer = vec![0.0; length].into_boxed_slice();
		Self { buffer, max: 0.0 }
	}

	/// Inserts a sample at the specified index in the buffer, updating the `max` field if necessary.
	/// TODO: What should happen if samples are inserted at the same position multiple times? Simply overwriting may
	///       cause `max` to be incorrect.
	pub fn insert_sample(&mut self, index: u32, value: f32) {
		self.buffer[index as usize] = value;
		self.max = self.max.max(value.abs());
	}
}

const SEND_DELAY: f64 = 0.005;

/// A struct containing sample data corresponding to a particular period of time.
#[derive(Debug)]
pub struct SampleBuffer {
	/// The sample data, split into individual channels.
	channels: [SampleBufferChannel; 8],
	/// The sample rate of the samples in the buffer.
	sample_rate: u32,
	/// The timestamp corresponding to the first sample in the buffer.
	start_time: SampleTime,
	/// The number of samples in the buffer. The buffer's end time can be calculated by multiplying this number by
	/// `sample_rate`.
	length: u32,
}

impl SampleBuffer {
	/// Creates a new sample buffer with the specified start time, length and sample rate. All samples are initialised
	/// to zero.
	pub fn new(sample_rate: u32, start_time: SampleTime, length: u32) -> Self {
		let channels = std::array::from_fn(|_| SampleBufferChannel::new(length as usize));
		Self {
			channels,
			sample_rate,
			start_time,
			length,
		}
	}

	/// Insert a sample into the buffer at the specified position.
	pub fn insert_sample(&mut self, smp_cnt: u32, sample: Sample) {
		let index = smp_cnt - self.start_time.samples(self.sample_rate);
		self.channels[0].insert_sample(index, sample.current_a);
		self.channels[1].insert_sample(index, sample.current_b);
		self.channels[2].insert_sample(index, sample.current_c);
		self.channels[3].insert_sample(index, sample.current_n);
		self.channels[4].insert_sample(index, sample.voltage_a);
		self.channels[5].insert_sample(index, sample.voltage_b);
		self.channels[6].insert_sample(index, sample.voltage_c);
		self.channels[7].insert_sample(index, sample.voltage_n);
	}

	/// Generates an OpenPMU XML sample datagram and sends it to the specified destination.
	/// TODO: Allow specifying destination
	/// TODO: Specific error type.
	pub fn flush(&self, out_skt: &UdpSocket) -> anyhow::Result<()> {
		let start_time_utc = OffsetDateTime::from_unix_timestamp(self.start_time.seconds(self.sample_rate) as i64)?
			+ Duration::from_secs_f32(self.start_time.samples(self.sample_rate) as f32 / self.sample_rate as f32);

		// TODO: Support nominal frequencies other than 50 Hz.
		// TODO: Actually, this can probably be changed to start.samples / length
		let frame = self.start_time.samples(self.sample_rate) * (NOMINAL_FREQUENCY * 2) / self.sample_rate;

		let (hours, minutes, seconds, microseconds) = start_time_utc.time().as_hms_micro();

		let mut buf = String::new();
		writeln!(&mut buf, "<OpenPMU>")?;
		writeln!(&mut buf, "\t<Format>Samples</Format>")?;
		writeln!(&mut buf, "\t<Date>{}</Date>", start_time_utc.date())?;
		writeln!(
			&mut buf,
			"\t<Time>{hours:02}:{minutes:02}:{seconds:02}.{microseconds:06}</Time>"
		)?;
		writeln!(&mut buf, "\t<Frame>{frame}</Frame>")?;
		writeln!(&mut buf, "\t<Fs>{}</Fs>", self.sample_rate)?;
		writeln!(&mut buf, "\t<n>{}</n>", self.length)?;
		writeln!(&mut buf, "\t<bits>16</bits>")?;
		writeln!(&mut buf, "\t<Channels>6</Channels>")?;

		fn build_channel(
			buf: &mut String,
			index: usize,
			name: &str,
			type_: &str,
			phase: &str,
			channel: &SampleBufferChannel,
		) -> anyhow::Result<()> {
			writeln!(buf, "\t<Channel_{index}>")?;
			writeln!(buf, "\t\t<Name>{name}</Name>")?;
			writeln!(buf, "\t\t<Type>{type_}</Type>")?;
			writeln!(buf, "\t\t<Phase>{phase}</Phase>")?;
			writeln!(buf, "\t\t<Range>{}</Range>", channel.max)?;

			let mut channel_bytes_buf = Vec::with_capacity(channel.buffer.len() * 2);
			if channel.max == 0.0 {
				channel_bytes_buf.resize(channel.buffer.len() * 2, 0);
			} else {
				for &value in &channel.buffer {
					let converted = (value / channel.max * 32767.0) as i16;
					channel_bytes_buf.extend(converted.to_be_bytes());
				}
			}

			write!(buf, "\t\t<Payload>")?;
			base64::engine::general_purpose::STANDARD.encode_string(&channel_bytes_buf, buf);
			writeln!(buf, "</Payload>")?;

			writeln!(buf, "\t</Channel_{index}>")?;
			Ok(())
		}

		build_channel(&mut buf, 0, "Belfast_Va", "V", "a", &self.channels[4])?;
		build_channel(&mut buf, 1, "Belfast_Vb", "V", "b", &self.channels[5])?;
		build_channel(&mut buf, 2, "Belfast_Vc", "V", "c", &self.channels[6])?;
		build_channel(&mut buf, 3, "Belfast_Ia", "I", "a", &self.channels[0])?;
		build_channel(&mut buf, 4, "Belfast_Ib", "I", "b", &self.channels[1])?;
		build_channel(&mut buf, 5, "Belfast_Ic", "I", "c", &self.channels[2])?;

		writeln!(&mut buf, "</OpenPMU>")?;

		out_skt.send_to(buf.as_bytes(), ("127.0.0.1", 48001))?;
		Ok(())
	}

	/// Given a sample timestamp, determines if it falls within this buffer's timespan.
	pub fn is_sample_within_timespan(&self, timestamp: SampleTime) -> bool {
		timestamp >= self.start_time && timestamp < self.start_time.add_samples(self.length)
	}

	/// Given a sample timestamp, determines if it comes after the end of this buffer's timespan.
	pub fn is_sample_after_timespan(&self, timestamp: SampleTime) -> bool {
		timestamp >= self.start_time.add_samples(self.length)
	}

	/// Calculates the time at which this buffer should be sent.
	pub fn get_send_time(&self) -> f64 {
		self.start_time.add_samples(self.length).to_secs_f64(self.sample_rate) + SEND_DELAY
	}
}

#[derive(Debug)]
struct SampleBufferManagerState {
	buffer_queue: Mutex<VecDeque<SampleBuffer>>,
	buffer_queue_cond: Condvar,
}

#[derive(Debug)]
pub struct SampleBufferManager {
	sample_rate: u32,
	buffer_length: u32,
	shared: Arc<SampleBufferManagerState>,
	_sender_thread: JoinHandle<()>,
}

const NS_PER_SEC: f64 = 1_000_000_000.0;

impl SampleBufferManager {
	pub fn new(sample_rate: u32, buffer_length: u32, out_socket: UdpSocket) -> Self {
		let shared = Arc::new(SampleBufferManagerState {
			buffer_queue: Mutex::new(VecDeque::new()),
			buffer_queue_cond: Condvar::new(),
		});

		let sender_shared = shared.clone();
		let sender_thread = std::thread::spawn(move || Self::sender_thread_fn(sender_shared, out_socket));

		Self {
			sample_rate,
			buffer_length,
			shared,
			_sender_thread: sender_thread,
		}
	}

	pub fn add_sample(&mut self, mut recv_time_s: u64, recv_time_ns: u32, asdu: Asdu) {
		let ns_per_sample = NS_PER_SEC / self.sample_rate as f64;
		let ns_offset = asdu.smp_cnt as f64 * ns_per_sample;

		if ns_offset >= recv_time_ns as f64 {
			recv_time_s -= 1;
		}

		let timestamp = SampleTime::from_seconds_and_samples(recv_time_s, asdu.smp_cnt as u32, self.sample_rate);

		let mut queue = self.shared.buffer_queue.lock().unwrap();
		if queue.back().map_or(true, |buffer| {
			buffer.is_sample_after_timespan(timestamp)
		}) {
			let mut new_buffer = SampleBuffer::new(
				self.sample_rate,
				SampleTime::from_seconds_and_samples(
					recv_time_s,
					asdu.smp_cnt as u32 / self.buffer_length * self.buffer_length,
					self.sample_rate,
				),
				self.buffer_length,
			);
			new_buffer.insert_sample(asdu.smp_cnt as u32, asdu.sample);
			queue.push_back(new_buffer);
			self.shared.buffer_queue_cond.notify_one();
		} else {
			let buffer = queue
				.iter_mut()
				.rev()
				.find(|buffer| buffer.is_sample_within_timespan(timestamp));

			if let Some(buffer) = buffer {
				buffer.insert_sample(asdu.smp_cnt as u32, asdu.sample);
			}
		}
	}

	fn sender_thread_fn(state: Arc<SampleBufferManagerState>, out_socket: UdpSocket) {
		loop {
			let sleep_time = {
				let queue = state
					.buffer_queue_cond
					.wait_while(state.buffer_queue.lock().unwrap(), |queue| queue.is_empty())
					.unwrap();

				queue.front().unwrap().get_send_time()
					- SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64()
			};

			if sleep_time > 0.0 {
				std::thread::sleep(Duration::from_secs_f64(sleep_time));
			}

			let buffer = {
				let mut queue = state.buffer_queue.lock().unwrap();
				queue.pop_front().unwrap()
			};

			buffer.flush(&out_socket).unwrap();
		}
	}
}
