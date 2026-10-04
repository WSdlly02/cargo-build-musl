use cargo_build_musl::cargo::{Project, host_cargo_home};
use cargo_build_musl::cli;
use cargo_build_musl::docker::{self, BuildSpec};
use cargo_build_musl::error;
use cargo_build_musl::error::{Error, Result};
use cargo_build_musl::platform::Platform;
use cargo_build_musl::verify;

use std::fs;
use std::process::ExitCode;
fn main() -> ExitCode {
    match run() {
        Ok(code) => exit_code(code),
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> error::Result<i32> {
    let cli = cli::Cli::parse_cargo(std::env::args_os().skip(1));

    let platform = Platform::host()?;
    let manifest_path = cli.manifest_path()?;
    let project = Project::discover(manifest_path.as_deref())?;
    let cargo_args = project.container_args(&cli.cargo_args);
    let cargo_home = host_cargo_home()?;

    create_directory(&project.target_directory)?;
    create_directory(&cargo_home)?;
    let user_mapping = docker::detect_user_mapping()?;

    let spec = BuildSpec {
        image: &cli.image,
        pull: cli.pull,
        user_mapping: &user_mapping,
        workspace_root: &project.workspace_root,
        target_directory: &project.target_directory,
        cargo_home: &cargo_home,
        container_workdir: &project.container_workdir,
        target: platform.target,
        cargo_args: &cargo_args,
        env: &cli.env,
    };
    let command = docker::build_command(&spec)?;

    if cli.dry_run {
        println!("{command:?}");
        return Ok(0);
    }

    eprintln!(
        "   Building {} in {}",
        platform.target,
        cli.image.to_string_lossy()
    );
    let result = docker::run_build(command, &project.target_directory)?;
    if result.status_code != 0 {
        return Ok(result.status_code);
    }

    verify::verify_all(&result.executables, platform)?;
    Ok(0)
}

fn create_directory(path: &std::path::Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|source| Error::CreateDirectory {
        path: path.to_owned(),
        source,
    })
}

fn exit_code(code: i32) -> ExitCode {
    match u8::try_from(code) {
        Ok(code) => ExitCode::from(code),
        Err(_) => ExitCode::FAILURE,
    }
}
