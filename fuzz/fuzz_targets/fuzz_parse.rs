#![no_main]

use libfuzzer_sys::fuzz_target;
use mu_rust::parse;

fuzz_target!(|data: &[u8]| {
	let _ = parse(data);
});
