use std::borrow::Cow;
use thiserror::Error;

pub type DeserializationResult<T> = std::result::Result<T, DeserializationError>;

// NOTE: only using this with T = () for now
// might adapt serialization to also return the number of bytes written
pub type SerializationResult<T> = std::result::Result<T, SerializationError>;

#[derive(Error, Debug)]
pub enum DeserializationError {
    #[error("invalid tag byte: {}", .0)]
    InvalidTag(u8),

    #[error("expected {:?} but found {:?}", .expected, .found)]
    UnexpectedTag {
        expected: crate::Tag,
        found: crate::Tag,
    },

    #[error("unexpected EOF")]
    EOF,

    #[error("field not found: {}", .0)]
    FieldNotFound(&'static str),

    #[error("duplicate value for field {}", .0)]
    FieldDuplicate(&'static str),

    #[error("Invalid UTF-8: {}", .0)]
    InvalidUTF8(#[from] std::str::Utf8Error),

    #[error("invalid i32 length (< 0)")]
    InvalidI32Length,

    #[error("list with element tag Tag::End but >0 elements")]
    InvalidList,

    #[error("{}", .0)]
    Custom(Cow<'static, str>),
}

#[derive(Error, Debug)]
pub enum SerializationError {
    #[error("string too long (len > u16::MAX)")]
    StringTooLong,

    #[error("list or array too long (len > i32::MAX)")]
    SeqTooLong,

    #[error("serialization error: {}", .0)]
    Custom(Cow<'static, str>),
}
