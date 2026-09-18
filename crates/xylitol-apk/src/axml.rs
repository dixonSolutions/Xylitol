//! Decoder for Android's binary XML ("AXML") container.
//!
//! An `AndroidManifest.xml` inside an APK is not text: it is a chunked binary
//! format sharing the resource-table string pool encoding. We only decode the
//! parts needed to describe a package, so this is deliberately a partial
//! reader rather than a general-purpose `aapt` replacement.

use std::collections::BTreeMap;

use crate::Error;

const RES_STRING_POOL_TYPE: u16 = 0x0001;
const RES_XML_START_ELEMENT: u16 = 0x0102;
const RES_XML_END_ELEMENT: u16 = 0x0103;

const UTF8_FLAG: u32 = 1 << 8;

// Value type codes from ResourceTypes.h.
const TYPE_REFERENCE: u8 = 0x01;
const TYPE_STRING: u8 = 0x03;
const TYPE_FLOAT: u8 = 0x04;
const TYPE_INT_DEC: u8 = 0x10;
const TYPE_INT_HEX: u8 = 0x11;
const TYPE_INT_BOOLEAN: u8 = 0x12;

/// A decoded attribute value. Values that reference the resource table are kept
/// as [`Value::Reference`] because resolving them needs `resources.arsc`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    Float(f32),
    Reference(u32),
    Raw(u32),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(v) => Some(*v),
            Value::Bool(v) => Some(*v as i64),
            _ => None,
        }
    }

    /// Human-readable rendering, used when surfacing values in the UI.
    pub fn to_display(&self) -> String {
        match self {
            Value::Str(s) => s.clone(),
            Value::Int(v) => v.to_string(),
            Value::Bool(v) => v.to_string(),
            Value::Float(v) => v.to_string(),
            Value::Reference(id) => format!("@0x{id:08x}"),
            Value::Raw(v) => format!("0x{v:08x}"),
        }
    }
}

/// One XML element with its attributes, keyed by local attribute name.
#[derive(Debug, Clone, Default)]
pub struct Element {
    pub name: String,
    pub attrs: BTreeMap<String, Value>,
    pub children: Vec<Element>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&Value> {
        self.attrs.get(name)
    }

    pub fn attr_str(&self, name: &str) -> Option<&str> {
        self.attr(name).and_then(Value::as_str)
    }

    pub fn attr_i64(&self, name: &str) -> Option<i64> {
        self.attr(name).and_then(Value::as_i64)
    }

    /// Direct children with the given tag name.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Element> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    pub fn child(&self, name: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == name)
    }
}

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Cursor { buf, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    fn u16(&mut self) -> Result<u16, Error> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn u32(&mut self) -> Result<u32, Error> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| Error::Axml("chunk length overflow".into()))?;
        if end > self.buf.len() {
            return Err(Error::Axml("truncated binary XML".into()));
        }
        let slice = &self.buf[self.pos..end];
        self.pos = end;
        Ok(slice)
    }
}

/// The string pool shared by every chunk in the document.
struct StringPool {
    strings: Vec<String>,
}

impl StringPool {
    fn get(&self, index: u32) -> Option<&str> {
        if index == u32::MAX {
            return None;
        }
        self.strings.get(index as usize).map(String::as_str)
    }

    fn parse(chunk: &[u8]) -> Result<StringPool, Error> {
        // `chunk` starts at the chunk header (type/headerSize/size).
        let mut c = Cursor::new(chunk);
        let _ty = c.u16()?;
        let _header_size = c.u16()?;
        let _size = c.u32()?;
        let string_count = c.u32()? as usize;
        let _style_count = c.u32()?;
        let flags = c.u32()?;
        let strings_start = c.u32()? as usize;
        let _styles_start = c.u32()?;

        let utf8 = flags & UTF8_FLAG != 0;
        let mut offsets = Vec::with_capacity(string_count);
        for _ in 0..string_count {
            offsets.push(c.u32()? as usize);
        }

        let mut strings = Vec::with_capacity(string_count);
        for off in offsets {
            let at = strings_start
                .checked_add(off)
                .ok_or_else(|| Error::Axml("string offset overflow".into()))?;
            if at >= chunk.len() {
                // Tolerate a damaged tail rather than failing the whole parse.
                strings.push(String::new());
                continue;
            }
            strings.push(Self::read_string(&chunk[at..], utf8).unwrap_or_default());
        }

        Ok(StringPool { strings })
    }

    fn read_string(buf: &[u8], utf8: bool) -> Option<String> {
        if utf8 {
            // UTF-8 pools store a character length then a byte length, each of
            // which is one byte unless the high bit marks a two-byte form.
            let (_, rest) = read_len8(buf)?;
            let (byte_len, rest) = read_len8(rest)?;
            let bytes = rest.get(..byte_len)?;
            Some(String::from_utf8_lossy(bytes).into_owned())
        } else {
            let (char_len, rest) = read_len16(buf)?;
            let bytes = rest.get(..char_len * 2)?;
            let units: Vec<u16> = bytes
                .chunks_exact(2)
                .map(|p| u16::from_le_bytes([p[0], p[1]]))
                .collect();
            Some(String::from_utf16_lossy(&units))
        }
    }
}

fn read_len8(buf: &[u8]) -> Option<(usize, &[u8])> {
    let first = *buf.first()? as usize;
    if first & 0x80 != 0 {
        let second = *buf.get(1)? as usize;
        Some((((first & 0x7f) << 8) | second, &buf[2..]))
    } else {
        Some((first, &buf[1..]))
    }
}

fn read_len16(buf: &[u8]) -> Option<(usize, &[u8])> {
    let first = u16::from_le_bytes([*buf.first()?, *buf.get(1)?]) as usize;
    if first & 0x8000 != 0 {
        let second = u16::from_le_bytes([*buf.get(2)?, *buf.get(3)?]) as usize;
        Some((((first & 0x7fff) << 16) | second, &buf[4..]))
    } else {
        Some((first, &buf[2..]))
    }
}

/// Decode a binary XML document into its root element.
pub fn parse(data: &[u8]) -> Result<Element, Error> {
    let mut c = Cursor::new(data);
    let _ty = c.u16()?;
    let header_size = c.u16()? as usize;
    let _file_size = c.u32()?;
    // Skip any extra header bytes a newer tool may have added.
    if header_size > 8 {
        c.take(header_size - 8)?;
    }

    let mut pool: Option<StringPool> = None;
    // Elements currently open, innermost last.
    let mut stack: Vec<Element> = Vec::new();
    let mut root: Option<Element> = None;

    while c.remaining() >= 8 {
        let chunk_start = c.pos;
        let ty = c.u16()?;
        let _chunk_header_size = c.u16()?;
        let chunk_size = c.u32()? as usize;
        if chunk_size < 8 {
            return Err(Error::Axml("chunk smaller than its header".into()));
        }
        let chunk_end = chunk_start
            .checked_add(chunk_size)
            .filter(|e| *e <= data.len())
            .ok_or_else(|| Error::Axml("chunk runs past end of file".into()))?;

        match ty {
            RES_STRING_POOL_TYPE => {
                pool = Some(StringPool::parse(&data[chunk_start..chunk_end])?);
            }
            RES_XML_START_ELEMENT => {
                let pool = pool
                    .as_ref()
                    .ok_or_else(|| Error::Axml("element before string pool".into()))?;
                let el = parse_start_element(&mut c, pool)?;
                stack.push(el);
            }
            RES_XML_END_ELEMENT => {
                if let Some(done) = stack.pop() {
                    match stack.last_mut() {
                        Some(parent) => parent.children.push(done),
                        None => root = Some(done),
                    }
                }
            }
            _ => {}
        }

        // Always resume at the declared chunk boundary: element chunks may carry
        // trailing data we do not read.
        c.pos = chunk_end;
    }

    root.or_else(|| stack.pop())
        .ok_or_else(|| Error::Axml("no root element".into()))
}

fn parse_start_element(c: &mut Cursor<'_>, pool: &StringPool) -> Result<Element, Error> {
    let _line = c.u32()?;
    let _comment = c.u32()?;
    let _ns = c.u32()?;
    let name_idx = c.u32()?;
    let attr_start = c.u16()? as usize;
    let attr_size = c.u16()? as usize;
    let attr_count = c.u16()? as usize;
    let _id_index = c.u16()?;
    let _class_index = c.u16()?;
    let _style_index = c.u16()?;

    let mut el = Element {
        name: pool.get(name_idx).unwrap_or_default().to_string(),
        ..Element::default()
    };

    // `attr_start` is measured from the start of the element body (which begins
    // right after the 8-byte chunk header).
    if attr_start > 20 {
        c.take(attr_start - 20)?;
    }
    if attr_size < 20 {
        return Err(Error::Axml("attribute entry too small".into()));
    }

    for _ in 0..attr_count {
        let _ns = c.u32()?;
        let name_idx = c.u32()?;
        let raw_value = c.u32()?;
        let _value_size = c.u16()?;
        let _res0 = c.take(1)?;
        let data_type = c.take(1)?[0];
        let data = c.u32()?;
        if attr_size > 20 {
            c.take(attr_size - 20)?;
        }

        let name = match pool.get(name_idx) {
            Some(n) if !n.is_empty() => n.to_string(),
            // Framework attributes are sometimes stored with an empty name and
            // identified only by their resource id; keep them addressable.
            _ => format!("attr0x{name_idx:08x}"),
        };

        let value = match data_type {
            TYPE_STRING => Value::Str(
                pool.get(data)
                    .or_else(|| pool.get(raw_value))
                    .unwrap_or_default()
                    .to_string(),
            ),
            TYPE_INT_DEC | TYPE_INT_HEX => Value::Int(data as i32 as i64),
            TYPE_INT_BOOLEAN => Value::Bool(data != 0),
            TYPE_FLOAT => Value::Float(f32::from_bits(data)),
            TYPE_REFERENCE => Value::Reference(data),
            _ => Value::Raw(data),
        };
        el.attrs.insert(name, value);
    }

    Ok(el)
}
