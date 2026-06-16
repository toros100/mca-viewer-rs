use std::borrow::Cow;

use thiserror::Error;

pub type DeserializationResult<T> = std::result::Result<T, DeserializationError>;
pub type SerializationResult<T> = std::result::Result<T, DeserializationError>;

#[derive(Error, Debug)]
pub enum DeserializationError {
    #[error("unexpected tag: {}", .0)]
    UnexpectedTag(u8),
    #[error("unexpected EOF")]
    EOF,
    #[error("field not found: {}", .0)]
    FieldNotFound(&'static str),
    #[error("duplicate value for field {}", .0)]
    FieldDuplicate(&'static str),
    #[error("string is not UTF-8")]
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
    #[error("string too long (> u16::MAX bytes)")]
    StringTooLong,
    #[error("list or array too long (> i32::MAX bytes)")]
    SeqTooLong,
    #[error("serialization error:")]
    Custom(Cow<'static, str>),
}
