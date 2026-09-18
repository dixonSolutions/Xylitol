//! The UTF-8 string pool path.
//!
//! Binary XML stores its strings either as UTF-16 or, when the high bit of the
//! pool flags is set, as UTF-8 with a two-part length prefix. Every manifest to
//! hand — F-Droid's and Flappy Bird's, eleven years apart — uses UTF-16, so the
//! UTF-8 branch would otherwise never run until it met a file in the wild and
//! quietly returned nonsense. These tests build such a document by hand.

use xylitol_apk::parse_manifest;

const RES_XML_TYPE: u16 = 0x0003;
const RES_STRING_POOL_TYPE: u16 = 0x0001;
const RES_XML_START_ELEMENT: u16 = 0x0102;
const RES_XML_END_ELEMENT: u16 = 0x0103;
const UTF8_FLAG: u32 = 1 << 8;

const TYPE_STRING: u8 = 0x03;
const TYPE_INT_DEC: u8 = 0x10;

const NO_ENTRY: u32 = u32::MAX;

/// Lengths below 0x80 take one byte; longer ones set the high bit and take two.
fn push_len(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(len as u8);
    } else {
        assert!(
            len < 0x8000,
            "fixture strings stay under the two-byte limit"
        );
        out.push(0x80 | (len >> 8) as u8);
        out.push((len & 0xff) as u8);
    }
}

fn utf8_string_pool(strings: &[&str]) -> Vec<u8> {
    let mut blob = Vec::new();
    let mut offsets = Vec::new();
    for s in strings {
        offsets.push(blob.len() as u32);
        // Character count first, then byte count: the two differ for anything
        // outside ASCII, which is the whole point of the distinction.
        push_len(&mut blob, s.chars().count());
        push_len(&mut blob, s.len());
        blob.extend_from_slice(s.as_bytes());
        blob.push(0);
    }
    while blob.len() % 4 != 0 {
        blob.push(0);
    }

    let header_size: u16 = 28;
    let strings_start = header_size as u32 + 4 * strings.len() as u32;
    let size = strings_start + blob.len() as u32;

    let mut out = Vec::new();
    out.extend(RES_STRING_POOL_TYPE.to_le_bytes());
    out.extend(header_size.to_le_bytes());
    out.extend(size.to_le_bytes());
    out.extend((strings.len() as u32).to_le_bytes());
    out.extend(0u32.to_le_bytes()); // style count
    out.extend(UTF8_FLAG.to_le_bytes());
    out.extend(strings_start.to_le_bytes());
    out.extend(0u32.to_le_bytes()); // styles start
    for offset in offsets {
        out.extend(offset.to_le_bytes());
    }
    out.extend_from_slice(&blob);
    out
}

/// `(name index, raw value index, type, data)`
type Attr = (u32, u32, u8, u32);

fn start_element(name: u32, attrs: &[Attr]) -> Vec<u8> {
    // chunk header (8) + node (8) + attrExt (20), then 20 bytes per attribute.
    let size = 36 + 20 * attrs.len() as u32;
    let mut out = Vec::new();
    out.extend(RES_XML_START_ELEMENT.to_le_bytes());
    out.extend(16u16.to_le_bytes()); // header size: chunk header + node
    out.extend(size.to_le_bytes());
    out.extend(1u32.to_le_bytes()); // line number
    out.extend(NO_ENTRY.to_le_bytes()); // comment
    out.extend(NO_ENTRY.to_le_bytes()); // namespace
    out.extend(name.to_le_bytes());
    out.extend(20u16.to_le_bytes()); // attributes start, from attrExt
    out.extend(20u16.to_le_bytes()); // attribute size
    out.extend((attrs.len() as u16).to_le_bytes());
    out.extend(0u16.to_le_bytes()); // id index
    out.extend(0u16.to_le_bytes()); // class index
    out.extend(0u16.to_le_bytes()); // style index
    for (name, raw, ty, data) in attrs {
        out.extend(NO_ENTRY.to_le_bytes()); // namespace
        out.extend(name.to_le_bytes());
        out.extend(raw.to_le_bytes());
        out.extend(8u16.to_le_bytes()); // value size
        out.push(0); // res0
        out.push(*ty);
        out.extend(data.to_le_bytes());
    }
    out
}

fn end_element(name: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend(RES_XML_END_ELEMENT.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(24u32.to_le_bytes());
    out.extend(1u32.to_le_bytes());
    out.extend(NO_ENTRY.to_le_bytes());
    out.extend(NO_ENTRY.to_le_bytes());
    out.extend(name.to_le_bytes());
    out
}

fn document(strings: &[&str], body: Vec<u8>) -> Vec<u8> {
    let pool = utf8_string_pool(strings);
    let total = 8 + pool.len() + body.len();
    let mut out = Vec::new();
    out.extend(RES_XML_TYPE.to_le_bytes());
    out.extend(8u16.to_le_bytes());
    out.extend((total as u32).to_le_bytes());
    out.extend_from_slice(&pool);
    out.extend_from_slice(&body);
    out
}

#[test]
fn a_utf8_pool_is_decoded_like_a_utf16_one() {
    let strings = [
        "manifest",
        "package",
        "com.example.utf8",
        "versionName",
        "1.0",
        "versionCode",
    ];
    let mut body = start_element(
        0,
        &[
            (1, 2, TYPE_STRING, 2),
            (3, 4, TYPE_STRING, 4),
            (5, NO_ENTRY, TYPE_INT_DEC, 42),
        ],
    );
    body.extend(end_element(0));

    let info = parse_manifest(&document(&strings, body)).expect("a UTF-8 pool should decode");
    assert_eq!(info.package, "com.example.utf8");
    assert_eq!(info.version_name.as_deref(), Some("1.0"));
    assert_eq!(info.version_code, Some(42));
}

#[test]
fn byte_length_and_character_length_are_not_confused() {
    // Each of these is longer in bytes than in characters, so a decoder that
    // reads the wrong one truncates the string or runs off the end of it.
    let package = "com.exämple.grüße";
    let version = "1.0-ünïcødé-🎉";
    let strings = ["manifest", "package", package, "versionName", version];
    let mut body = start_element(0, &[(1, 2, TYPE_STRING, 2), (3, 4, TYPE_STRING, 4)]);
    body.extend(end_element(0));

    let info = parse_manifest(&document(&strings, body)).unwrap();
    assert_ne!(
        package.len(),
        package.chars().count(),
        "fixture is not multi-byte"
    );
    assert_eq!(info.package, package);
    assert_eq!(info.version_name.as_deref(), Some(version));
}

#[test]
fn strings_longer_than_127_bytes_use_the_two_byte_length() {
    let long_version = "9.".repeat(150); // 300 bytes, past the one-byte limit
    let strings = [
        "manifest",
        "package",
        "com.example.long",
        "versionName",
        &long_version,
    ];
    let mut body = start_element(0, &[(1, 2, TYPE_STRING, 2), (3, 4, TYPE_STRING, 4)]);
    body.extend(end_element(0));

    let info = parse_manifest(&document(&strings, body)).unwrap();
    assert_eq!(info.package, "com.example.long");
    assert_eq!(info.version_name.as_deref(), Some(long_version.as_str()));
}
