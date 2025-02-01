use std::{
	collections::VecDeque,
	fmt::Write,
	net::UdpSocket,
	sync::{Arc, Condvar, Mutex},
	thread::JoinHandle,
	time::{Duration, SystemTime, UNIX_EPOCH}
};

use base64::Engine;
use time::OffsetDateTime;

use crate::{Asdu, Sample};

#[derive(Debug)]
pub struct SampleBufferChannel {
	buffer: Box<[f32]>,
	max: f32,
}

impl SampleBufferChannel {
	pub fn new(length: usize) -> Self {
		let buffer = vec![0.0; length].into_boxed_slice();
		Self {
			buffer,
			max: 0.0,
		}
	}

	pub fn add_sample(&mut self, index: usize, value: f32) {
		self.buffer[index] = value;
		self.max = self.max.max(value.abs());
	}
}

#[derive(Debug)]
pub struct SampleBuffer {
	channels: [SampleBufferChannel; 8],
	sample_rate: u32,
	start_time_s: i64,
	sample_offset: usize,
	length: usize,
}

impl SampleBuffer {
	pub fn new(
		sample_rate: u32,
		start_time_s: i64,
		sample_offset: usize,
		length: usize,
	) -> Self {
		let channels = std::array::from_fn(|_| SampleBufferChannel::new(length));
		Self {
			channels,
			sample_rate,
			start_time_s,
			sample_offset,
			length,
		}
	}

	pub fn add_sample(&mut self, smp_cnt: usize, sample: Sample) {
		let index = smp_cnt - self.sample_offset;
		self.channels[0].add_sample(index, sample.current_a);
		self.channels[1].add_sample(index, sample.current_b);
		self.channels[2].add_sample(index, sample.current_c);
		self.channels[3].add_sample(index, sample.current_n);
		self.channels[4].add_sample(index, sample.voltage_a);
		self.channels[5].add_sample(index, sample.voltage_b);
		self.channels[6].add_sample(index, sample.voltage_c);
		self.channels[7].add_sample(index, sample.voltage_n);
	}

	pub fn flush(&self, out_skt: &UdpSocket) -> anyhow::Result<()> {
		let start_time_utc = OffsetDateTime::from_unix_timestamp(self.start_time_s).unwrap()
			+ Duration::from_secs_f32(self.sample_offset as f32 / self.sample_rate as f32);

		// TODO: Support nominal frequencies other than 50 Hz.
		let frame = self.sample_offset * 100 / self.sample_rate as usize;

		let (hours, minutes, seconds, microseconds) = start_time_utc.time().as_hms_micro();

		let mut buf = String::new();
		writeln!(&mut buf, "<OpenPMU>")?;
		writeln!(&mut buf, "\t<Format>Samples</Format>")?;
		writeln!(&mut buf, "\t<Date>{}</Date>", start_time_utc.date())?;
		writeln!(&mut buf, "\t<Time>{hours:02}:{minutes:02}:{seconds:02}.{microseconds:06}</Time>")?;
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

	pub fn is_sample_within_timespan(&self, seconds: i64, count: u32) -> bool {
		let buffer_start_time = self.start_time_s * self.sample_rate as i64 + self.sample_offset as i64;
		let buffer_end_time = buffer_start_time + self.length as i64;
		let sample_time = seconds * self.sample_rate as i64 + count as i64;
		buffer_start_time <= sample_time && sample_time < buffer_end_time
	}

	pub fn is_sample_after_timespan(&self, seconds: i64, count: u32) -> bool {
		let buffer_end_time = self.start_time_s * self.sample_rate as i64 + self.sample_offset as i64 + self.length as i64;
		let sample_time = seconds * self.sample_rate as i64 + count as i64;
		sample_time >= buffer_end_time
	}

	pub fn get_send_time(&self) -> f64 {
		self.start_time_s as f64 + (self.sample_offset + self.length) as f64 / self.sample_rate as f64 + 0.005
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
	buffer_length: usize,
	shared: Arc<SampleBufferManagerState>,
	_sender_thread: JoinHandle<()>,
}

const NS_PER_SEC: f64 = 1_000_000_000.0;

impl SampleBufferManager {

	pub fn new(sample_rate: u32, buffer_length: usize, out_socket: UdpSocket) -> Self {
	
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

	pub fn add_sample(&mut self, mut recv_time_s: i64, recv_time_ns: u32, asdu: Asdu) {

		let ns_per_sample = NS_PER_SEC / self.sample_rate as f64;
		let ns_offset = asdu.smp_cnt as f64 * ns_per_sample;

		if ns_offset >= recv_time_ns as f64 {
			recv_time_s -= 1;
		}

		let mut queue = self.shared.buffer_queue.lock().unwrap();
		if queue.back().map_or(true, |buffer| buffer.is_sample_after_timespan(recv_time_s, asdu.smp_cnt as u32)) {
			let mut new_buffer = SampleBuffer::new(self.sample_rate, recv_time_s, asdu.smp_cnt as usize / self.buffer_length * self.buffer_length, self.buffer_length);
			new_buffer.add_sample(asdu.smp_cnt as usize, asdu.sample);
			queue.push_back(new_buffer);
			self.shared.buffer_queue_cond.notify_one();
		} else {
			let buffer = queue.iter_mut()
				.rev()
				.find(|buffer| buffer.is_sample_within_timespan(recv_time_s, asdu.smp_cnt as u32));

			if let Some(buffer) = buffer {
				buffer.add_sample(asdu.smp_cnt as usize, asdu.sample);
			}
		}

	}

	fn sender_thread_fn(state: Arc<SampleBufferManagerState>, out_socket: UdpSocket) {
		loop {
			let sleep_time = {
				let queue = state.buffer_queue_cond.wait_while(
					state.buffer_queue.lock().unwrap(),
					|queue| queue.is_empty()
				).unwrap();

				queue.front().unwrap().get_send_time() - SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64()
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
