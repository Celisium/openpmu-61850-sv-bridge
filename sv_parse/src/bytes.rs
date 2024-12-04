use thiserror::Error;

#[derive(Debug, Clone)]
pub struct BytesReader<'b> {
	bytes: &'b [u8]
}

#[derive(Debug, Error)]
pub enum BytesReaderError {
	#[error("Unexpected end of buffer")]
	EndOfBuffer
}

impl<'b> BytesReader<'b> {

	pub fn new(bytes: &'b [u8]) -> Self {
		Self { bytes }
	}

	pub fn peek_bytes(&self, length: usize) -> Result<&'b [u8], BytesReaderError> {
		self.bytes.get(..length).ok_or(BytesReaderError::EndOfBuffer)
	}

	pub fn read_bytes(&mut self, length: usize) -> Result<&'b [u8], BytesReaderError> {
		let (read, remaining) = self.bytes.split_at_checked(length).ok_or(BytesReaderError::EndOfBuffer)?;
		self.bytes = remaining;
		Ok(read)
	}

	pub fn read_u8_array<const N: usize>(&mut self) -> Result<[u8; N], BytesReaderError> {
		// slice has length N, so conversion to [u8; N] will always succeed.
		self.read_bytes(N).map(|slice| slice.try_into().unwrap())
	}

	pub fn peek_sub_reader(&mut self, length: usize) -> Result<Self, BytesReaderError> {
		self.peek_bytes(length).map(Self::new)
	}

	pub fn take_sub_reader(&mut self, length: usize) -> Result<Self, BytesReaderError> {
		self.read_bytes(length).map(Self::new)
	}

	pub fn limit(&mut self, length: usize) -> Result<(), BytesReaderError> {
		self.bytes = self.bytes.get(..length).ok_or(BytesReaderError::EndOfBuffer)?;
		Ok(())
	}

	pub fn skip(&mut self, length: usize) -> Result<(), BytesReaderError> {
		self.bytes = self.bytes.get(length..).ok_or(BytesReaderError::EndOfBuffer)?;
		Ok(())
	}
	
	pub fn peek_u8(&self) -> Result<u8, BytesReaderError> {
		self.bytes.first().ok_or(BytesReaderError::EndOfBuffer).copied()
	}

	pub fn read_u8(&mut self) -> Result<u8, BytesReaderError> {
		let &value = self.bytes.first().ok_or(BytesReaderError::EndOfBuffer)?;
		self.bytes = &self.bytes[1..];
		Ok(value)
	}

	pub fn read_u16_be(&mut self) -> Result<u16, BytesReaderError> {
		self.read_u8_array().map(u16::from_be_bytes)
	}

	pub fn is_empty(&self) -> bool {
		self.bytes.is_empty()
	}
}
