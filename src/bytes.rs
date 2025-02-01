use thiserror::Error;

#[derive(Debug, Clone)]
pub struct BytesReader<'b> {
	bytes: &'b [u8],
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum BytesReaderError {
	#[error("Unexpected end of buffer")]
	EndOfBuffer,
}

impl<'b> BytesReader<'b> {
	pub fn new(bytes: &'b [u8]) -> Self {
		Self { bytes }
	}

	pub fn peek_bytes(&self, length: usize) -> Result<&'b [u8], BytesReaderError> {
		self.bytes.get(..length).ok_or(BytesReaderError::EndOfBuffer)
	}

	pub fn read_bytes(&mut self, length: usize) -> Result<&'b [u8], BytesReaderError> {
		let (read, remaining) = self
			.bytes
			.split_at_checked(length)
			.ok_or(BytesReaderError::EndOfBuffer)?;
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn is_empty() {
		let reader = BytesReader::new(&[]);
		assert!(reader.is_empty());
	}

	#[test]
	fn read_valid() {
		let mut reader = BytesReader::new(b"test string");
		assert_eq!(reader.read_bytes(4), Ok(b"test".as_slice()));
		assert_eq!(reader.read_bytes(0), Ok(b"".as_slice()));
		assert_eq!(reader.read_bytes(7), Ok(b" string".as_slice()));
		assert!(reader.is_empty());
	}

	#[test]
	fn read_past_end() {
		let mut reader = BytesReader::new(b"test string");
		assert_eq!(reader.read_bytes(4), Ok(b"test".as_slice()));
		assert_eq!(reader.read_bytes(8), Err(BytesReaderError::EndOfBuffer));
		assert_eq!(reader.read_bytes(7), Ok(b" string".as_slice()));
		assert_eq!(reader.read_bytes(10), Err(BytesReaderError::EndOfBuffer));
		assert!(reader.is_empty());
		assert_eq!(reader.read_bytes(0), Ok(b"".as_slice()));
	}

	#[test]
	fn peek_valid() {
		let mut reader = BytesReader::new(b"test string");
		assert_eq!(reader.peek_bytes(4), Ok(b"test".as_slice()));
		assert_eq!(reader.read_bytes(4), Ok(b"test".as_slice()));
		assert_eq!(reader.peek_bytes(0), Ok(b"".as_slice()));
		assert_eq!(reader.read_bytes(7), Ok(b" string".as_slice()));
		assert!(reader.is_empty());
	}

	#[test]
	fn peek_past_end() {
		let mut reader = BytesReader::new(b"test string");
		assert_eq!(reader.read_bytes(4), Ok(b"test".as_slice()));
		assert_eq!(reader.peek_bytes(8), Err(BytesReaderError::EndOfBuffer));
		assert_eq!(reader.peek_bytes(7), Ok(b" string".as_slice()));
		assert_eq!(reader.read_bytes(7), Ok(b" string".as_slice()));
		assert_eq!(reader.peek_bytes(10), Err(BytesReaderError::EndOfBuffer));
		assert!(reader.is_empty());
		assert_eq!(reader.peek_bytes(0), Ok(b"".as_slice()));
	}

	#[test]
	fn limit_valid() {
		let mut reader = BytesReader::new(b"valid limit test");
		assert_eq!(reader.read_bytes(6), Ok(b"valid ".as_slice()));
		assert_eq!(reader.limit(5), Ok(()));
		assert_eq!(reader.read_bytes(3), Ok(b"lim".as_slice()));
		assert_eq!(reader.peek_bytes(3), Err(BytesReaderError::EndOfBuffer));
		assert_eq!(reader.read_bytes(3), Err(BytesReaderError::EndOfBuffer));
		assert_eq!(reader.peek_bytes(2), Ok(b"it".as_slice()));
		assert_eq!(reader.read_bytes(2), Ok(b"it".as_slice()));
		assert!(reader.is_empty());
		assert_eq!(reader.peek_bytes(1), Err(BytesReaderError::EndOfBuffer));
		assert_eq!(reader.read_bytes(1), Err(BytesReaderError::EndOfBuffer));
		assert_eq!(reader.peek_bytes(0), Ok(b"".as_slice()));
		assert_eq!(reader.read_bytes(0), Ok(b"".as_slice()));
	}

	#[test]
	fn limit_past_end() {
		let mut reader = BytesReader::new(b"invalid limit test");
		assert_eq!(reader.read_bytes(8), Ok(b"invalid ".as_slice()));
		assert_eq!(reader.limit(11), Err(BytesReaderError::EndOfBuffer));
		assert_eq!(reader.peek_bytes(3), Ok(b"lim".as_slice()));
		assert_eq!(reader.read_bytes(3), Ok(b"lim".as_slice()));
		assert_eq!(reader.limit(7), Ok(()));
		assert_eq!(reader.read_bytes(7), Ok(b"it test".as_slice()));
		assert_eq!(reader.limit(1), Err(BytesReaderError::EndOfBuffer));
	}

	#[test]
	fn sub_reader() {
		let mut reader = BytesReader::new(b"sub reader test");
		assert_eq!(reader.read_bytes(4), Ok(b"sub ".as_slice()));

		let mut sub_reader = reader.take_sub_reader(6).unwrap();
		assert_eq!(sub_reader.read_bytes(3), Ok(b"rea".as_slice()));
		assert_eq!(sub_reader.read_bytes(4), Err(BytesReaderError::EndOfBuffer));
		assert_eq!(sub_reader.read_bytes(3), Ok(b"der".as_slice()));
		assert_eq!(sub_reader.read_bytes(1), Err(BytesReaderError::EndOfBuffer));
		assert_eq!(sub_reader.read_bytes(0), Ok(b"".as_slice()));

		reader
			.peek_sub_reader(6)
			.expect_err("should fail with partially out of bounds length");
		reader
			.take_sub_reader(6)
			.expect_err("should fail with partially out of bounds length");

		let mut sub_reader = reader.peek_sub_reader(5).unwrap();
		assert_eq!(sub_reader.read_bytes(5), Ok(b" test".as_slice()));
		assert_eq!(reader.read_bytes(5), Ok(b" test".as_slice()));

		reader
			.peek_sub_reader(1)
			.expect_err("should fail with fully out of bounds length");
		reader
			.take_sub_reader(1)
			.expect_err("should fail with fully out of bounds length");

		reader.peek_sub_reader(0).expect("should succeed with zero length");
		reader.take_sub_reader(0).expect("should succeed with zero length");
	}

	#[test]
	fn read_u8_valid() {
		let mut reader = BytesReader::new(&[1, 1, 2, 3, 5, 8]);
		assert_eq!(reader.read_u8(), Ok(1));
		assert_eq!(reader.read_u8(), Ok(1));
		assert_eq!(reader.read_u8(), Ok(2));
		assert_eq!(reader.peek_u8(), Ok(3));
		assert_eq!(reader.read_u8(), Ok(3));
		assert_eq!(reader.read_u8(), Ok(5));
		assert_eq!(reader.read_u8(), Ok(8));
	}

	#[test]
	fn read_u8_past_end() {
		let mut reader = BytesReader::new(&[5, 10, 15]);
		assert_eq!(reader.read_u8(), Ok(5));
		assert_eq!(reader.read_u8(), Ok(10));
		assert_eq!(reader.read_u8(), Ok(15));
		assert_eq!(reader.peek_u8(), Err(BytesReaderError::EndOfBuffer));
		assert_eq!(reader.read_u8(), Err(BytesReaderError::EndOfBuffer));
	}

	#[test]
	fn read_u16_be() {
		let mut reader = BytesReader::new(&[0x12, 0x34, 0x56]);
		assert_eq!(reader.read_u16_be(), Ok(0x1234));
		assert_eq!(reader.read_u16_be(), Err(BytesReaderError::EndOfBuffer));
	}
}
