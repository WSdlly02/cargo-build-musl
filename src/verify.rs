use std::fs;
use std::path::{Path, PathBuf};

use goblin::elf::Elf;

use crate::error::{Error, Result};
use crate::platform::Platform;

pub fn verify_all(paths: &[PathBuf], platform: Platform) -> Result<()> {
    for path in paths {
        verify(path, platform)?;
    }
    Ok(())
}

fn verify(path: &Path, platform: Platform) -> Result<()> {
    let bytes = fs::read(path).map_err(|source| Error::ReadArtifact {
        path: path.to_owned(),
        source,
    })?;
    let elf = Elf::parse(&bytes).map_err(|source| Error::InvalidElf {
        path: path.to_owned(),
        source,
    })?;

    if elf.header.e_machine != platform.elf_machine {
        return Err(Error::Message(format!(
            "artifact `{}` has ELF machine {}, expected {}",
            path.display(),
            elf.header.e_machine,
            platform.elf_machine
        )));
    }

    if let Some(interpreter) = elf.interpreter {
        return Err(Error::Message(format!(
            "artifact `{}` is dynamically linked through `{interpreter}`",
            path.display()
        )));
    }

    if !elf.libraries.is_empty() {
        return Err(Error::Message(format!(
            "artifact `{}` has dynamic dependencies: {}",
            path.display(),
            elf.libraries.join(", ")
        )));
    }

    eprintln!("    Verified static ELF {}", path.display());
    Ok(())
}
