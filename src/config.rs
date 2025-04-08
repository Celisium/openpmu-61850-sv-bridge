use std::net::SocketAddr;
use serde::Deserialize;

use crate::ethernet::MacAddress;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputChannelType {
	Voltage,
	Current,
}

#[derive(Deserialize)]
pub struct OutputChannel {
	pub name: String,
	pub phase: String,
	#[serde(rename = "type")]
	pub type_: OutputChannelType,
	pub input_channel: usize,
}

#[derive(Deserialize)]
pub struct Configuration {
	pub nominal_frequency: u32,
	pub sample_rate: u32,
	pub interface: String,
	#[serde(rename = "output_channel")]
	pub channels: Vec<OutputChannel>,
	pub destination: SocketAddr,
	pub mac_address: MacAddress,
}
