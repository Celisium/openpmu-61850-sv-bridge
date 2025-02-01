use thiserror::Error;

use crate::bytes::{BytesReader, BytesReaderError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
	Universal(u32),
	Application(u32),
	ContextSpecific(u32),
	Private(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
	Primitive,
	Constructed,
}

// TODO: This structure should be compressed into 4 bytes (which would still give 29 bits for the tag).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identifier {
	pub tag: Tag,
	pub encoding: Encoding,
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum DecodeError {
	#[error("Encountered an unexpected tag")]
	UnexpectedTag,
	#[error("Tag number exceeds the maximum supported value")]
	TagOutOfRange,
	#[error("Indefinite length is not supported")]
	IndefiniteLength,
	#[error("Encountered a reserved length value")]
	ReservedLength,
	#[error("Length exceeds the maximum supported value")]
	LengthOutOfRange,
	#[error("Invalid integer encoding")]
	InvalidIntegerEncoding,
	#[error("Integer is out of range")]
	IntegerOutOfRange,
	#[error("Constructed strings are not supported")]
	ConstructedString,
	#[error("Invalid VisibleString")]
	InvalidVisibleString,
	#[error(transparent)]
	ReadError(#[from] BytesReaderError),
}

pub fn read_identifier(reader: &mut BytesReader<'_>) -> Result<Identifier, DecodeError> {

	let first_byte = reader.read_u8()?;

	let encoding = if (first_byte & (1 << 5)) == 0 {
		Encoding::Primitive
	} else {
		Encoding::Constructed
	};

	let num = match first_byte & 0b0001_1111 {
		31 => {
			let mut num = 0;
			loop {
				let next_byte = reader.read_u8()?;

				num <<= 7;
				num |= (next_byte & 0b0111_1111) as u32;

				if (next_byte & (1 << 7)) == 0 {
					break;
				}
				
				if num.leading_zeros() < 7 {
					return Err(DecodeError::TagOutOfRange);
				}
			}
			num
		},
		num => num as u32,
	};

	let tag = match first_byte >> 6 {
		0 => Tag::Universal(num),
		1 => Tag::Application(num),
		2 => Tag::ContextSpecific(num),
		3 => Tag::Private(num),
		_ => unreachable!(),
	};

	Ok(Identifier { tag, encoding })
}

pub fn read_required_identifier(reader: &mut BytesReader<'_>, tag: Tag) -> Result<Encoding, DecodeError> {
	let identifier = read_identifier(reader)?;
	if identifier.tag == tag {
		Ok(identifier.encoding)
	} else {
		Err(DecodeError::UnexpectedTag)
	}
}

pub fn read_optional_identifier(reader: &mut BytesReader<'_>, tag: Tag) -> Result<Option<Encoding>, DecodeError> {
	if reader.is_empty() {
		return Ok(None);
	}

	let mut peek_reader = reader.clone();
	let identifier = read_identifier(&mut peek_reader)?;
	if identifier.tag == tag {
		*reader = peek_reader;
		Ok(Some(identifier.encoding))
	} else {
		Ok(None)
	}
}

pub fn read_length(reader: &mut BytesReader<'_>) -> Result<usize, DecodeError> {
	match reader.read_u8()? {
		// Definite form, short
		value @ 0..0b1000_0000 => Ok(value as usize),

		// Indefinite form (not supported)
		0b1000_0000 => Err(DecodeError::IndefiniteLength),

		// Reserved
		0b1111_1111 => Err(DecodeError::ReservedLength),

		// Definite form, long
		value => {
			let mut length: usize = 0;
			for _ in 0..(value & 0b0111_1111) {

				if length.leading_zeros() < 8 {
					return Err(DecodeError::LengthOutOfRange);
				}

				length <<= 8;
				length |= reader.read_u8()? as usize;
			}
			Ok(length)
		},
	}
}

pub fn read_integer_as_u16(reader: &mut BytesReader<'_>, encoding: Encoding) -> Result<u16, DecodeError> {

	if encoding != Encoding::Primitive {
		return Err(DecodeError::InvalidIntegerEncoding);
	}

	let length = read_length(reader)?;

	match *reader.read_bytes(length)? {
		// Integers must contain at least one byte.
		[] => Err(DecodeError::InvalidIntegerEncoding),

		// Overlong encodings (those where the first nine bits are the same) are invalid.
		[0, ..0x80, ..] => Err(DecodeError::InvalidIntegerEncoding),
		[0xFF, (0x80..), ..] => Err(DecodeError::InvalidIntegerEncoding),

		// Negative values are out of range for a u16.
		[(0x80..), ..] => Err(DecodeError::IntegerOutOfRange),

		// 1 byte encoding (0..=127)
		[b_0] => Ok(b_0 as u16),

		// 2 byte encoding (128..=32767)
		[b_0, b_1] => Ok(u16::from_be_bytes([b_0, b_1])),

		// 3 byte encoding (32768..65535)
		[0, b_0, b_1] => Ok(u16::from_be_bytes([b_0, b_1])),

		// Any other valid encoding would be out of range for a u16.
		_ => Err(DecodeError::IntegerOutOfRange),
	}

}

pub fn read_octet_string<'b>(reader: &mut BytesReader<'b>, encoding: Encoding) -> Result<&'b [u8], DecodeError> {
	if encoding == Encoding::Constructed {
		return Err(DecodeError::ConstructedString);
	}

	let length = read_length(reader)?;
	reader.read_bytes(length).map_err(Into::into)
}

pub fn read_visiblestring<'b>(reader: &mut BytesReader<'b>, encoding: Encoding) -> Result<&'b str, DecodeError> {
	if encoding == Encoding::Constructed {
		return Err(DecodeError::ConstructedString);
	}

	let length = read_length(reader)?;

	let bytes = reader.read_bytes(length)?;

	// TODO: Confirm that this is the correct range for VisibleString.
	let valid = bytes.iter()
		.all(|b| (0x20..=0x7E).contains(b));

	if valid {
		Ok(std::str::from_utf8(bytes).unwrap())
	} else {
		Err(DecodeError::InvalidVisibleString)
	}
}

#[cfg(test)]
mod tests {
	#![allow(clippy::unusual_byte_groupings)]
	use super::*;

	#[test]
	fn read_identifier_valid() {
		let bytes = [
			0b00_0_01010,
			0b01_0_10101,
			0b10_0_11100,
			0b11_0_00111,
			0b00_1_01010,
			0b01_1_10101,
			0b10_1_11100,
			0b11_1_00111,
			0b00_0_11111, 0x8A, 0x55,
			0b01_1_11111, 0x80, 0x80, 0x55,
			0b10_0_11111, 0x81, 0xCD, 0xAF, 0x9B, 0x6F,
		];

		let mut reader = BytesReader::new(&bytes);

		let result = read_identifier(&mut reader);
		assert_eq!(result, Ok(Identifier { tag: Tag::Universal(10), encoding: Encoding::Primitive }));
		let result = read_identifier(&mut reader);
		assert_eq!(result, Ok(Identifier { tag: Tag::Application(21), encoding: Encoding::Primitive }));
		let result = read_identifier(&mut reader);
		assert_eq!(result, Ok(Identifier { tag: Tag::ContextSpecific(28), encoding: Encoding::Primitive }));
		let result = read_identifier(&mut reader);
		assert_eq!(result, Ok(Identifier { tag: Tag::Private(7), encoding: Encoding::Primitive }));

		let result = read_identifier(&mut reader);
		assert_eq!(result, Ok(Identifier { tag: Tag::Universal(10), encoding: Encoding::Constructed }));
		let result = read_identifier(&mut reader);
		assert_eq!(result, Ok(Identifier { tag: Tag::Application(21), encoding: Encoding::Constructed }));
		let result = read_identifier(&mut reader);
		assert_eq!(result, Ok(Identifier { tag: Tag::ContextSpecific(28), encoding: Encoding::Constructed }));
		let result = read_identifier(&mut reader);
		assert_eq!(result, Ok(Identifier { tag: Tag::Private(7), encoding: Encoding::Constructed }));

		let result = read_identifier(&mut reader);
		assert_eq!(result, Ok(Identifier { tag: Tag::Universal(0x555), encoding: Encoding::Primitive }));
		let result = read_identifier(&mut reader);
		assert_eq!(result, Ok(Identifier { tag: Tag::Application(0x55), encoding: Encoding::Constructed }));
		let result = read_identifier(&mut reader);
		assert_eq!(result, Ok(Identifier { tag: Tag::ContextSpecific(0x19ABCDEF), encoding: Encoding::Primitive }));

		assert!(reader.is_empty());
	}

	#[test]
	fn read_identifier_out_of_range() {
		let mut reader = BytesReader::new(&[0b10_0_11111, 0x91, 0xCD, 0xAF, 0x9B, 0x6F]);
		assert_eq!(read_identifier(&mut reader), Err(DecodeError::TagOutOfRange));
	}

	#[test]
	fn read_required_identifier_expected() {
		let bytes = [
			0b10_0_01010,
			0b10_1_11111, 0x8A, 0x55,
		];

		let mut reader = BytesReader::new(&bytes);
		let result = read_required_identifier(&mut reader, Tag::ContextSpecific(10));
		assert_eq!(result, Ok(Encoding::Primitive));

		let result = read_required_identifier(&mut reader, Tag::ContextSpecific(0x555));
		assert_eq!(result, Ok(Encoding::Constructed));
		assert!(reader.is_empty());
	}

	#[test]
	fn read_required_identifier_unexpected() {
		let mut reader = BytesReader::new(&[0b10_0_01010]);
		let result = read_required_identifier(&mut reader, Tag::ContextSpecific(9));
		assert_eq!(result, Err(DecodeError::UnexpectedTag));

		let mut reader = BytesReader::new(&[0b10_1_11111, 0x8A, 0x55]);
		let result = read_required_identifier(&mut reader, Tag::ContextSpecific(0xAAA));
		assert_eq!(result, Err(DecodeError::UnexpectedTag));
	}

	#[test]
	fn read_optional_identifier_present() {
		let bytes = [
			0b10_0_01010,
			0b10_1_11111, 0x8A, 0x55,
		];

		let mut reader = BytesReader::new(&bytes);
		let result = read_optional_identifier(&mut reader, Tag::ContextSpecific(10));
		assert_eq!(result, Ok(Some(Encoding::Primitive)));

		let result = read_optional_identifier(&mut reader, Tag::ContextSpecific(0x555));
		assert_eq!(result, Ok(Some(Encoding::Constructed)));
		assert!(reader.is_empty());
	}

	#[test]
	fn read_optional_identifier_absent() {
		let mut reader = BytesReader::new(&[0b10_0_01010]);
		let result = read_optional_identifier(&mut reader, Tag::ContextSpecific(9));
		assert_eq!(result, Ok(None));

		let result = read_optional_identifier(&mut reader, Tag::ContextSpecific(10));
		assert_eq!(result, Ok(Some(Encoding::Primitive)));
		assert!(reader.is_empty());

		let result = read_optional_identifier(&mut reader, Tag::ContextSpecific(11));
		assert_eq!(result, Ok(None));
	}

	#[test]
	fn read_length_valid() {
		let bytes = [
			0x12,
			0x82, 0x12, 0x34,
			0x84, 0x12, 0x34, 0x56, 0x78,
			0x89, 0x00, 0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0,
		];

		let mut reader = BytesReader::new(&bytes);

		let result = read_length(&mut reader);
		assert_eq!(result, Ok(0x12));

		let result = read_length(&mut reader);
		assert_eq!(result, Ok(0x1234));

		let result = read_length(&mut reader);
		assert_eq!(result, Ok(0x12345678));

		let result = read_length(&mut reader);
		assert_eq!(result, Ok(0x123456789ABCDEF0));

		assert!(reader.is_empty());
	}

	#[test]
	fn read_length_indefinite() {
		let mut reader = BytesReader::new(&[0x80]);
		let result = read_length(&mut reader);
		assert_eq!(result, Err(DecodeError::IndefiniteLength));
	}

	#[test]
	fn read_length_reserved() {
		let mut reader = BytesReader::new(&[0xFF]);
		let result = read_length(&mut reader);
		assert_eq!(result, Err(DecodeError::ReservedLength));
	}

	#[test]
	fn read_length_out_of_range() {
		let mut reader = BytesReader::new(&[0x89, 0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0, 0x12]);
		let result = read_length(&mut reader);
		assert_eq!(result, Err(DecodeError::LengthOutOfRange));
	}

	#[test]
	fn read_length_invalid_length() {
		let mut reader = BytesReader::new(&[0x85, 0x12, 0x34, 0x56, 0x78]);
		read_length(&mut reader)
			.expect_err("should fail when reader runs out of bytes");
	}

	#[test]
	fn read_integer_as_u16_valid() {
		let bytes = [
			0x01, 0x12,
			0x02, 0x34, 0x56,
			0x03, 0x00, 0x89, 0xAB,
		];
		let mut reader = BytesReader::new(&bytes);

		let result = read_integer_as_u16(&mut reader, Encoding::Primitive);
		assert_eq!(result, Ok(0x12));

		let result = read_integer_as_u16(&mut reader, Encoding::Primitive);
		assert_eq!(result, Ok(0x3456));

		let result = read_integer_as_u16(&mut reader, Encoding::Primitive);
		assert_eq!(result, Ok(0x89AB));

		assert!(reader.is_empty());
	}

	#[test]
	fn read_integer_as_u16_constructed() {
		let mut reader = BytesReader::new(&[0x01, 0x12]);
		read_integer_as_u16(&mut reader, Encoding::Constructed)
			.expect_err("should fail with constructed length");
	}

	#[test]
	fn read_integer_as_u16_zero_length() {
		let mut reader = BytesReader::new(&[0x00]);
		read_integer_as_u16(&mut reader, Encoding::Primitive)
			.expect_err("should fail with length of zero");
	}

	#[test]
	fn read_integer_as_u16_overlong() {
		let mut reader = BytesReader::new(&[0x02, 0x00, 0x12]);
		read_integer_as_u16(&mut reader, Encoding::Primitive)
			.expect_err("should fail with overlong encoding");

		let mut reader = BytesReader::new(&[0x02, 0xFF, 0x89]);
		read_integer_as_u16(&mut reader, Encoding::Primitive)
			.expect_err("should fail with overlong encoding");
	}

	#[test]
	fn read_integer_as_u16_out_of_range() {
		let mut reader = BytesReader::new(&[0x02, 0x89, 0xAB]);
		read_integer_as_u16(&mut reader, Encoding::Primitive)
			.expect_err("should fail with negative value");

		let mut reader = BytesReader::new(&[0x03, 0x12, 0x34, 0x56]);
		read_integer_as_u16(&mut reader, Encoding::Primitive)
			.expect_err("should fail with value which is out of range");
	}

	#[test]
	fn read_octet_string_valid() {
		let mut reader = BytesReader::new(b"\x06abc\x00\x01\x02");
		let result = read_octet_string(&mut reader, Encoding::Primitive)
			.unwrap();
		assert_eq!(result, b"abc\x00\x01\x02");
		assert!(reader.is_empty());
	}

	#[test]
	fn read_octet_string_invalid_length() {
		let mut reader = BytesReader::new(b"\x07abc\x00\x01\x02");
		read_octet_string(&mut reader, Encoding::Primitive)
			.expect_err("should fail with invalid length");
	}

	#[test]
	fn read_octet_string_constructed() {
		let mut reader = BytesReader::new(b"\x06abc\x00\x01\x02");
		read_octet_string(&mut reader, Encoding::Constructed)
			.expect_err("should fail with constructed tag");
	}

	#[test]
	fn read_visiblestring_valid() {
		let mut reader = BytesReader::new(b"\x04test");
		let result = read_visiblestring(&mut reader, Encoding::Primitive)
			.unwrap();
		assert_eq!(result, "test");
		assert!(reader.is_empty());

		let mut reader = BytesReader::new(b"\x03test");
		let result = read_visiblestring(&mut reader, Encoding::Primitive)
			.unwrap();
		assert_eq!(result, "tes");
		assert!(reader.skip(1).is_ok()); // There should be exactly one byte remaining.
		assert!(reader.is_empty());
	}

	#[test]
	fn read_visiblestring_invalid_length() {
		let mut reader = BytesReader::new(b"\x05test");
		read_visiblestring(&mut reader, Encoding::Primitive)
			.expect_err("should fail with invalid length");
	}

	#[test]
	fn read_visiblestring_constructed() {
		let mut reader = BytesReader::new(b"\x04test");
		read_visiblestring(&mut reader, Encoding::Constructed)
			.expect_err("should fail with constructed tag");
	}

	#[test]
	fn read_visiblestring_invalid_chars() {
		// ASCII control characters
		let mut bytes = b"\x08control\ncharacter".to_owned();
		bytes[0] = bytes.len() as u8;
		for c in (0x00..0x20).chain(std::iter::once(0xFF)) {
			bytes[8] = c;
			let mut reader = BytesReader::new(&bytes);
			read_visiblestring(&mut reader, Encoding::Primitive)
				.expect_err("should fail with ASCII control characters");
		}

		// Non-ASCII characters
		let mut reader = BytesReader::new(b"\x05caf\xC3\xA9"); // 'café' in UTF-8
		read_visiblestring(&mut reader, Encoding::Primitive)
			.expect_err("should fail with non-ASCII characters");
	}
}
