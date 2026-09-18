//! Reading Android native objects.
//!
//! An Android `.so` is an ordinary ELF shared object, so the format is not the
//! problem: what it *links against* is. It names `libc.so`, `libandroid.so`,
//! `liblog.so` and friends, and those are bionic and the Android framework, not
//! anything on a Linux desktop. This module answers what an object needs, so
//! [`crate::symbols`] can decide what to do about each name.

use std::collections::BTreeSet;

use object::read::elf::{Dyn, FileHeader, SectionHeader};
use object::{Endianness, Object, ObjectSymbol};
use serde::{Deserialize, Serialize};

use crate::abi::Abi;
use crate::Error;

/// What one native object needs from its host.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeObject {
    /// The file name inside the APK, e.g. `lib/x86_64/libroblox.so`.
    pub path: String,
    /// `DT_SONAME`, when the object declares one.
    pub soname: Option<String>,
    pub abi: Abi,
    /// `DT_NEEDED` entries, in link order.
    pub needed: Vec<String>,
    /// Every undefined symbol the object imports.
    pub undefined: Vec<String>,
    /// Entry points that say how the app expects to be started.
    pub entry_points: Vec<EntryPoint>,
    /// Count of `Java_*` symbols: JNI natives registered by name.
    pub jni_natives: usize,
    pub file_size: u64,
}

/// How an app hands control to its native code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EntryPoint {
    /// `ANativeActivity_onCreate` — the NDK's NativeActivity. The whole app is
    /// native; the Java side is a stock shim the platform provides.
    NativeActivity,
    /// `GameActivity_onCreate` — the AGDK successor to NativeActivity.
    GameActivity,
    /// `JNI_OnLoad` — the object is a library called *from* Java. Something has
    /// to run that Java for this to mean anything.
    JniOnLoad,
    /// `SDL_main` or `android_main` — an app framework's own entry.
    AndroidMain,
}

impl EntryPoint {
    pub fn symbol(self) -> &'static str {
        match self {
            EntryPoint::NativeActivity => "ANativeActivity_onCreate",
            EntryPoint::GameActivity => "GameActivity_onCreate",
            EntryPoint::JniOnLoad => "JNI_OnLoad",
            EntryPoint::AndroidMain => "android_main",
        }
    }

    /// Whether this entry point alone is enough to start the app.
    ///
    /// `JNI_OnLoad` is not: it is called by a Java VM that has already loaded
    /// the app's classes. Without one, nothing ever calls it.
    pub fn is_self_sufficient(self) -> bool {
        !matches!(self, EntryPoint::JniOnLoad)
    }

    fn all() -> [EntryPoint; 4] {
        [
            EntryPoint::NativeActivity,
            EntryPoint::GameActivity,
            EntryPoint::JniOnLoad,
            EntryPoint::AndroidMain,
        ]
    }
}

/// Read one native object from bytes.
pub fn read(path: &str, data: &[u8]) -> Result<NativeObject, Error> {
    let file = object::File::parse(data).map_err(|e| Error::Elf {
        path: path.to_string(),
        reason: e.to_string(),
    })?;

    let abi = match file.architecture() {
        object::Architecture::X86_64 => Abi::X86_64,
        object::Architecture::I386 => Abi::X86,
        object::Architecture::Aarch64 => Abi::Arm64V8a,
        object::Architecture::Arm => Abi::ArmeabiV7a,
        other => {
            return Err(Error::Elf {
                path: path.to_string(),
                reason: format!("unsupported architecture {other:?}"),
            })
        }
    };

    let (soname, needed) = dynamic_entries(path, data)?;

    let mut undefined = BTreeSet::new();
    let mut jni_natives = 0usize;
    let mut defined = BTreeSet::new();
    for symbol in file.dynamic_symbols() {
        let Ok(name) = symbol.name() else { continue };
        if name.is_empty() {
            continue;
        }
        if symbol.is_undefined() {
            undefined.insert(name.to_string());
        } else {
            if name.starts_with("Java_") {
                jni_natives += 1;
            }
            defined.insert(name.to_string());
        }
    }

    let entry_points = EntryPoint::all()
        .into_iter()
        .filter(|e| defined.contains(e.symbol()))
        .collect();

    Ok(NativeObject {
        path: path.to_string(),
        soname,
        abi,
        needed,
        undefined: undefined.into_iter().collect(),
        entry_points,
        jni_natives,
        file_size: data.len() as u64,
    })
}

/// Read `DT_SONAME` and `DT_NEEDED` out of the dynamic section.
///
/// `object`'s high-level API does not expose these, so this walks the dynamic
/// table itself. Both 32- and 64-bit objects appear in real APKs, so both
/// widths are handled.
fn dynamic_entries(path: &str, data: &[u8]) -> Result<(Option<String>, Vec<String>), Error> {
    let fail = |reason: String| Error::Elf {
        path: path.to_string(),
        reason,
    };

    // The ELF class byte says which width to parse as.
    let class = *data.get(4).ok_or_else(|| fail("file too short".into()))?;
    match class {
        object::elf::ELFCLASS64 => {
            let header = object::elf::FileHeader64::<Endianness>::parse(data)
                .map_err(|e| fail(e.to_string()))?;
            dynamic_of(header, data).map_err(fail)
        }
        object::elf::ELFCLASS32 => {
            let header = object::elf::FileHeader32::<Endianness>::parse(data)
                .map_err(|e| fail(e.to_string()))?;
            dynamic_of(header, data).map_err(fail)
        }
        other => Err(fail(format!("unknown ELF class {other}"))),
    }
}

fn dynamic_of<Elf: FileHeader<Endian = Endianness>>(
    header: &Elf,
    data: &[u8],
) -> Result<(Option<String>, Vec<String>), String> {
    let endian = header.endian().map_err(|e| e.to_string())?;
    let sections = header.sections(endian, data).map_err(|e| e.to_string())?;

    let mut soname = None;
    let mut needed = Vec::new();

    for section in sections.iter() {
        let Ok(Some((entries, index))) = section.dynamic(endian, data) else {
            continue;
        };
        let strings = sections
            .strings(endian, data, index)
            .map_err(|e| e.to_string())?;
        for entry in entries {
            let tag = entry.d_tag(endian).into();
            match tag as u32 {
                object::elf::DT_SONAME => {
                    if let Ok(name) = entry.string(endian, strings) {
                        soname = Some(String::from_utf8_lossy(name).into_owned());
                    }
                }
                object::elf::DT_NEEDED => {
                    if let Ok(name) = entry.string(endian, strings) {
                        needed.push(String::from_utf8_lossy(name).into_owned());
                    }
                }
                _ => {}
            }
        }
    }

    Ok((soname, needed))
}
