use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use cargo_metadata::MetadataCommand;

use crate::error::{Error, Result};

#[derive(Debug)]
pub struct Project {
    pub workspace_root: PathBuf,
    pub target_directory: PathBuf,
    pub container_workdir: String,
    pub container_manifest: Option<PathBuf>,
}

impl Project {
    pub fn discover(manifest_path: Option<&Path>) -> Result<Self> {
        let mut command = MetadataCommand::new();
        command.no_deps();
        if let Some(path) = manifest_path {
            command.manifest_path(path);
        }

        let metadata = command.exec()?;
        let workspace_root = metadata.workspace_root.into_std_path_buf();
        let target_directory = metadata.target_directory.into_std_path_buf();
        let current_directory = env::current_dir().map_err(|source| Error::Spawn {
            program: "current_dir",
            source,
        })?;
        let container_manifest = manifest_path
            .map(|path| {
                let canonical = path.canonicalize().map_err(|error| {
                    Error::Message(format!(
                        "cannot resolve manifest `{}`: {error}",
                        path.display()
                    ))
                })?;
                workspace_path(&workspace_root, &canonical)
            })
            .transpose()?;
        // Preserve member selection when invoked inside the workspace. Outside
        // callers use the explicit manifest's directory as their working dir.
        let workdir = match current_directory.strip_prefix(&workspace_root) {
            Ok(relative) => Path::new("/workspace").join(relative),
            Err(_) => container_manifest
                .as_ref()
                .and_then(|path| path.parent())
                .ok_or_else(|| {
                    Error::Message("outside-workspace builds require --manifest-path".into())
                })?
                .to_owned(),
        };

        let container_workdir = workdir
            .to_str()
            .ok_or_else(|| Error::Message("workspace path is not valid UTF-8".into()))?
            .to_owned();

        Ok(Self {
            workspace_root,
            target_directory,
            container_workdir,
            container_manifest,
        })
    }

    pub fn container_args(&self, args: &[OsString]) -> Vec<OsString> {
        let Some(manifest) = &self.container_manifest else {
            return args.to_vec();
        };
        let mut result = Vec::new();
        let mut args = args.iter();
        while let Some(arg) = args.next() {
            if arg == "--manifest-path" {
                args.next();
                result.push(arg.clone());
                result.push(manifest.as_os_str().to_owned());
            } else if arg.as_encoded_bytes().starts_with(b"--manifest-path=") {
                result.push(OsString::from("--manifest-path"));
                result.push(manifest.as_os_str().to_owned());
            } else {
                result.push(arg.clone());
            }
        }
        result
    }
}

fn workspace_path(root: &Path, path: &Path) -> Result<PathBuf> {
    let relative = path.strip_prefix(root).map_err(|_| {
        Error::Message(format!(
            "manifest `{}` is outside mounted workspace `{}`",
            path.display(),
            root.display()
        ))
    })?;
    Ok(Path::new("/workspace").join(relative))
}

pub fn host_cargo_home() -> Result<PathBuf> {
    if let Some(path) = env::var_os("CARGO_HOME") {
        return absolutize(PathBuf::from(path));
    }

    let home = env::var_os("HOME").ok_or_else(|| {
        Error::Message("neither CARGO_HOME nor HOME is set; cannot locate Cargo cache".to_owned())
    })?;
    absolutize(PathBuf::from(home).join(".cargo"))
}

fn absolutize(path: PathBuf) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path);
    }

    let current_directory = env::current_dir().map_err(|source| Error::Spawn {
        program: "current_dir",
        source,
    })?;
    Ok(current_directory.join(path))
}
