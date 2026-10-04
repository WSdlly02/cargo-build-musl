use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{0}")]
    Message(String),

    #[error("cargo metadata failed: {0}")]
    CargoMetadata(#[from] cargo_metadata::Error),

    #[error("failed to run `{program}`: {source}")]
    Spawn {
        program: &'static str,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to read output from the build container: {0}")]
    BuildOutput(#[source] std::io::Error),

    #[error("Cargo emitted an invalid JSON message: {0}")]
    CargoMessage(#[source] std::io::Error),

    #[error("failed to create directory `{path}`: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("failed to read artifact `{path}`: {source}")]
    ReadArtifact {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("artifact `{path}` is not a valid ELF file: {source}")]
    InvalidElf {
        path: PathBuf,
        #[source]
        source: goblin::error::Error,
    },
}
