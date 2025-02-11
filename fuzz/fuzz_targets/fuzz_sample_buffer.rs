#![no_main]

use std::net::UdpSocket;

use libfuzzer_sys::{
	arbitrary::{Arbitrary, Unstructured},
	fuzz_target,
};
use mu_rust::{sample_buffer::SampleBufferManager, Asdu, Sample};

#[derive(Debug)]
struct AsduWrapper(Asdu);

impl Arbitrary<'_> for AsduWrapper {
	fn arbitrary(u: &mut Unstructured<'_>) -> libfuzzer_sys::arbitrary::Result<Self> {
		Ok(Self(Asdu {
			svid: u.arbitrary()?,
			datset: u.arbitrary()?,
			smp_cnt: u.arbitrary()?,
			conf_rev: u.arbitrary()?,
			refr_tm: u.arbitrary()?,
			smp_synch: u.arbitrary()?,
			smp_rate: u.arbitrary()?,
			sample: Sample {
				current_a: u.arbitrary()?,
				current_b: u.arbitrary()?,
				current_c: u.arbitrary()?,
				current_n: u.arbitrary()?,
				voltage_a: u.arbitrary()?,
				voltage_b: u.arbitrary()?,
				voltage_c: u.arbitrary()?,
				voltage_n: u.arbitrary()?,
			},
			smp_mod: u.arbitrary()?,
		}))
	}
}

fuzz_target!(|data: Vec<AsduWrapper>| {
	let socket = UdpSocket::bind(("127.0.0.1", 0)).unwrap();
	let mut sample_buffer_manager = SampleBufferManager::new(4000, 40, socket);

	let mut ns = 156255;

	for AsduWrapper(asdu) in data {
		sample_buffer_manager.insert_sample(1_000_000_000, ns, asdu);
		ns += 1000;
	}
});
