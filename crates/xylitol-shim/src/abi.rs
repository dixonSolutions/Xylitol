//! Android ABIs, and which of them this machine can run.

use serde::{Deserialize, Serialize};

/// An Android ABI name, as it appears under `lib/` in an APK.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Abi {
    Arm64V8a,
    ArmeabiV7a,
    X86,
    X86_64,
}

impl Abi {
    pub fn as_str(self) -> &'static str {
        match self {
            Abi::Arm64V8a => "arm64-v8a",
            Abi::ArmeabiV7a => "armeabi-v7a",
            Abi::X86 => "x86",
            Abi::X86_64 => "x86_64",
        }
    }

    pub fn parse(s: &str) -> Option<Abi> {
        match s.trim() {
            "arm64-v8a" | "arm64_v8a" => Some(Abi::Arm64V8a),
            "armeabi-v7a" | "armeabi_v7a" => Some(Abi::ArmeabiV7a),
            "x86" => Some(Abi::X86),
            "x86_64" => Some(Abi::X86_64),
            _ => None,
        }
    }

    /// The ELF machine type this ABI's objects declare.
    pub fn elf_machine(self) -> u16 {
        match self {
            Abi::Arm64V8a => object::elf::EM_AARCH64,
            Abi::ArmeabiV7a => object::elf::EM_ARM,
            Abi::X86 => object::elf::EM_386,
            Abi::X86_64 => object::elf::EM_X86_64,
        }
    }

    pub fn is_64_bit(self) -> bool {
        matches!(self, Abi::Arm64V8a | Abi::X86_64)
    }
}

impl std::fmt::Display for Abi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The ABI this build of Xylitol can load code for.
///
/// The shim maps an app's native objects into its own address space, so the
/// instruction set has to be the host's. There is no translation layer: an
/// arm64 app on an x86-64 desktop needs an emulator, which is the thing this
/// exists to avoid.
pub fn host() -> Option<Abi> {
    match std::env::consts::ARCH {
        "x86_64" => Some(Abi::X86_64),
        "x86" => Some(Abi::X86),
        "aarch64" => Some(Abi::Arm64V8a),
        "arm" => Some(Abi::ArmeabiV7a),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_names_and_lib_dir_names_both_parse() {
        // `lib/arm64-v8a/` in an APK, `config.arm64_v8a.apk` in a bundle.
        assert_eq!(Abi::parse("arm64-v8a"), Some(Abi::Arm64V8a));
        assert_eq!(Abi::parse("arm64_v8a"), Some(Abi::Arm64V8a));
        assert_eq!(Abi::parse("mips"), None);
    }

    #[test]
    fn this_machine_has_a_usable_abi() {
        let host = host().expect("tests run on a supported architecture");
        assert!(host.is_64_bit() || host == Abi::X86);
    }
}
