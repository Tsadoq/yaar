//! A path travels as the bytes the system knows it by (not necessarily UTF-8).
//! Field layout of a frame, little endian. Text and byte runs: u32 length, then bytes (text is
//! UTF-8). List: u32 count, then items. Optional: one byte 0 or 1, then the value if 1.
//! A frame other than the hello holds exactly its fields; leftover bytes are refused.

use std::ffi::OsString;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use crate::LinkError;

#[derive(Default)]
pub(crate) struct Out(pub(crate) Vec<u8>);

impl Out {
    pub(crate) fn u8(&mut self, value: u8) {
        self.0.push(value);
    }

    pub(crate) fn u16(&mut self, value: u16) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_le_bytes());
    }

    pub(crate) fn flag(&mut self, value: bool) {
        self.u8(u8::from(value));
    }

    pub(crate) fn bytes(&mut self, value: &[u8]) {
        // A run longer than a u32 can count is longer than any frame; the whole-frame length check
        // refuses it.
        self.u32(u32::try_from(value.len()).unwrap_or(u32::MAX));
        self.0.extend_from_slice(value);
    }

    pub(crate) fn text(&mut self, value: &str) {
        self.bytes(value.as_bytes());
    }

    pub(crate) fn path(&mut self, value: &Path) {
        self.bytes(value.as_os_str().as_bytes());
    }

    pub(crate) fn list<T>(&mut self, items: &[T], mut each: impl FnMut(&mut Out, &T)) {
        self.u32(u32::try_from(items.len()).unwrap_or(u32::MAX));
        for item in items {
            each(self, item);
        }
    }

    pub(crate) fn optional<T: ?Sized>(
        &mut self,
        value: Option<&T>,
        write: impl FnOnce(&mut Out, &T),
    ) {
        match value {
            None => self.u8(0),
            Some(value) => {
                self.u8(1);
                write(self, value);
            }
        }
    }
}

pub(crate) struct In<'a> {
    rest: &'a [u8],
}

impl<'a> In<'a> {
    pub(crate) fn new(payload: &'a [u8]) -> Self {
        Self { rest: payload }
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], LinkError> {
        if self.rest.len() < count {
            return Err(LinkError::Malformed("the frame ends inside a field"));
        }
        let (taken, rest) = self.rest.split_at(count);
        self.rest = rest;
        Ok(taken)
    }

    pub(crate) fn u8(&mut self) -> Result<u8, LinkError> {
        Ok(self.take(1)?[0])
    }

    pub(crate) fn u16(&mut self) -> Result<u16, LinkError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    pub(crate) fn u32(&mut self) -> Result<u32, LinkError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    pub(crate) fn u64(&mut self) -> Result<u64, LinkError> {
        let bytes = self.take(8)?;
        let mut value = [0u8; 8];
        value.copy_from_slice(bytes);
        Ok(u64::from_le_bytes(value))
    }

    pub(crate) fn flag(&mut self) -> Result<bool, LinkError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(LinkError::Malformed("a flag is neither 0 nor 1")),
        }
    }

    pub(crate) fn bytes(&mut self) -> Result<Vec<u8>, LinkError> {
        let len = self.u32()? as usize;
        Ok(self.take(len)?.to_vec())
    }

    pub(crate) fn text(&mut self) -> Result<String, LinkError> {
        String::from_utf8(self.bytes()?).map_err(|_| LinkError::Malformed("a text is not UTF-8"))
    }

    pub(crate) fn path(&mut self) -> Result<PathBuf, LinkError> {
        Ok(PathBuf::from(OsString::from_vec(self.bytes()?)))
    }

    pub(crate) fn list<T>(
        &mut self,
        mut each: impl FnMut(&mut In<'a>) -> Result<T, LinkError>,
    ) -> Result<Vec<T>, LinkError> {
        let count = self.u32()? as usize;
        // Every item takes at least one byte, so a count beyond the bytes left is false; refuse it
        // before allocating for the items.
        if count > self.rest.len() {
            return Err(LinkError::Malformed("a list is longer than its frame"));
        }
        let mut items = Vec::with_capacity(count);
        for _ in 0..count {
            items.push(each(self)?);
        }
        Ok(items)
    }

    pub(crate) fn optional<T>(
        &mut self,
        read: impl FnOnce(&mut In<'a>) -> Result<T, LinkError>,
    ) -> Result<Option<T>, LinkError> {
        match self.u8()? {
            0 => Ok(None),
            1 => read(self).map(Some),
            _ => Err(LinkError::Malformed("a flag is neither 0 nor 1")),
        }
    }

    pub(crate) fn end(self) -> Result<(), LinkError> {
        if self.rest.is_empty() {
            Ok(())
        } else {
            Err(LinkError::Malformed("the frame is longer than its fields"))
        }
    }
}
