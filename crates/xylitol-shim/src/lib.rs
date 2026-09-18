pub mod abi;
pub mod bionic;
pub mod elf;
pub mod report;
pub mod symbols;
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path}: {reason}")]
    Elf { path: String, reason: String },
    #[error("could not read the package: {0}")]
    Apk(String),
}
