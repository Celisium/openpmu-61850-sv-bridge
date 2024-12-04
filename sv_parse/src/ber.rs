use core::str;

use thiserror::Error;

use crate::bytes::{BytesReader, BytesReaderError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
	Universal(u32),
	Application(u32),
	ContextSpecific(u32),
	Private(u32)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
	Primitive,
	Constructed
}

#[derive(Debug, Clone, Copy)]
pub struct Identifier {
	pub tag: Tag,
	pub encoding: Encoding,
}

#[derive(Debug, Error)]
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
	ReadError(#[from] BytesReaderError)
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
		num => num as u32
	};

	let tag = match first_byte >> 6 {
		0 => Tag::Universal(num),
		1 => Tag::Application(num),
		2 => Tag::ContextSpecific(num),
		3 => Tag::Private(num),
		_ => unreachable!()
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
		}
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
		_ => Err(DecodeError::IntegerOutOfRange)
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

	// TODO: Validate character set (for VisibleString it is ASCII 0x20 to 0x7E)
	str::from_utf8(bytes).map_err(|_| DecodeError::InvalidVisibleString)
}

