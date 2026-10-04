use crate::error::{Error, Result};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Platform {
    pub target: &'static str,
    pub elf_machine: u16,
}

impl Platform {
    pub fn host() -> Result<Self> {
        if !cfg!(target_os = "linux") {
            return Err(Error::Message(
                "cargo-build-musl currently supports Linux hosts only".to_owned(),
            ));
        }

        Self::from_arch(std::env::consts::ARCH).ok_or_else(|| {
            Error::Message(format!(
                "unsupported host architecture `{}`; expected x86_64 or aarch64",
                std::env::consts::ARCH
            ))
        })
    }

    fn from_arch(arch: &str) -> Option<Self> {
        match arch {
            "x86_64" => Some(Self {
                target: "x86_64-unknown-linux-musl",
                elf_machine: goblin::elf::header::EM_X86_64,
            }),
            "aarch64" => Some(Self {
                target: "aarch64-unknown-linux-musl",
                elf_machine: goblin::elf::header::EM_AARCH64,
            }),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_host_architectures_to_musl_targets() {
        assert_eq!(
            Platform::from_arch("x86_64").unwrap().target,
            "x86_64-unknown-linux-musl"
        );
        assert_eq!(
            Platform::from_arch("aarch64").unwrap().target,
            "aarch64-unknown-linux-musl"
        );
        assert!(Platform::from_arch("riscv64").is_none());
    }
}
