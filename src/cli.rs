use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use clap::{CommandFactory, Parser, ValueEnum, error::ErrorKind};

use crate::error::{Error, Result};

pub const DEFAULT_IMAGE: &str = "rust:alpine";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum PullPolicy {
    #[default]
    Missing,
    Always,
    Never,
}

impl PullPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Always => "always",
            Self::Never => "never",
        }
    }
}

#[derive(Debug, Parser)]
#[command(
    name = "cargo-build-musl",
    bin_name = "cargo build-musl",
    version,
    about = "Build a static musl binary in an official Rust Alpine container",
    trailing_var_arg = true,
    after_help = "All trailing arguments are forwarded unchanged to `cargo build`.\n\
                  Put build-musl options before the first Cargo build option.\n\
                  The --target, --target-dir, and --message-format options are managed internally."
)]
pub struct Cli {
    /// Container image to use
    #[arg(
        long = "build-musl-image",
        value_name = "IMAGE",
        default_value = DEFAULT_IMAGE
    )]
    pub image: OsString,

    /// Docker pull policy
    #[arg(
        long = "build-musl-pull",
        value_name = "POLICY",
        value_enum,
        default_value = "missing"
    )]
    pub pull: PullPolicy,

    /// Print the Docker command without running it
    #[arg(long = "build-musl-dry-run")]
    pub dry_run: bool,

    /// Pass a host environment variable to the container (repeatable)
    #[arg(long = "build-musl-env", value_name = "NAME", value_parser = env_name)]
    pub env: Vec<String>,

    /// Options forwarded to `cargo build`
    #[arg(value_name = "CARGO_BUILD_OPTIONS", allow_hyphen_values = true)]
    pub cargo_args: Vec<OsString>,
}

impl Cli {
    pub fn parse_cargo<I>(args: I) -> Self
    where
        I: IntoIterator<Item = OsString>,
    {
        match Self::try_parse_cargo(args) {
            Ok(cli) => cli,
            Err(error) => error.exit(),
        }
    }

    pub fn try_parse_cargo<I>(args: I) -> std::result::Result<Self, clap::Error>
    where
        I: IntoIterator<Item = OsString>,
    {
        let mut args = args.into_iter().peekable();
        if args.peek().is_some_and(|arg| arg == "build-musl") {
            args.next();
        }

        let cli = <Self as Parser>::try_parse_from(
            std::iter::once(OsString::from("cargo build-musl")).chain(args),
        )?;
        cli.validate_managed_options()?;
        Ok(cli)
    }

    pub fn manifest_path(&self) -> Result<Option<PathBuf>> {
        let mut args = self.cargo_args.iter();

        while let Some(arg) = args.next() {
            if arg == "--manifest-path" {
                return args
                    .next()
                    .map(PathBuf::from)
                    .map(Some)
                    .ok_or_else(|| Error::Message("`--manifest-path` requires a path".to_owned()));
            }
            if let Some(value) = option_value(arg, "--manifest-path=") {
                return Ok(Some(PathBuf::from(value)));
            }
        }

        Ok(None)
    }

    fn validate_managed_options(&self) -> std::result::Result<(), clap::Error> {
        const OPTIONS: [&str; 3] = ["--target", "--target-dir", "--message-format"];

        for arg in &self.cargo_args {
            let Some(arg) = arg.to_str() else {
                continue;
            };
            for option in OPTIONS {
                if arg == option || arg.starts_with(&format!("{option}=")) {
                    let mut command = Self::command();
                    return Err(command.error(
                        ErrorKind::ArgumentConflict,
                        format!("`{option}` is managed by cargo-build-musl and cannot be supplied"),
                    ));
                }
            }
        }

        Ok(())
    }
}

fn option_value(arg: &OsStr, prefix: &str) -> Option<OsString> {
    let value = arg.to_str()?.strip_prefix(prefix)?;
    Some(OsString::from(value))
}

fn env_name(value: &str) -> std::result::Result<String, String> {
    let mut chars = value.chars();
    if !chars
        .next()
        .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        || !chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
    {
        return Err("expected an environment variable name, such as RUSTFLAGS".into());
    }
    if matches!(value, "CARGO_HOME" | "CARGO_TARGET_DIR") {
        return Err(format!("{value} is managed by cargo-build-musl"));
    }
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> std::result::Result<Cli, clap::Error> {
        Cli::try_parse_cargo(args.iter().map(OsString::from))
    }

    #[test]
    fn strips_cargo_subcommand_and_forwards_build_arguments() {
        let cli = parse(&[
            "build-musl",
            "--release",
            "--features",
            "tls",
            "-p",
            "server",
        ])
        .unwrap();

        assert_eq!(
            cli.cargo_args,
            ["--release", "--features", "tls", "-p", "server"]
                .map(OsString::from)
                .to_vec()
        );
    }

    #[test]
    fn parses_plugin_options_without_forwarding_them() {
        let cli = parse(&[
            "--build-musl-image=rust:custom",
            "--build-musl-pull",
            "never",
            "--build-musl-dry-run",
            "--release",
        ])
        .unwrap();

        assert_eq!(cli.image, "rust:custom");
        assert_eq!(cli.pull, PullPolicy::Never);
        assert!(cli.dry_run);
        assert_eq!(cli.cargo_args, [OsString::from("--release")]);
    }

    #[test]
    fn rejects_invalid_pull_policy_with_clap_error() {
        let error = parse(&["--build-musl-pull", "sometimes"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidValue);
    }

    #[test]
    fn rejects_cargo_options_owned_by_the_plugin() {
        let error = parse(&["--target=x86_64-unknown-linux-gnu"]).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::ArgumentConflict);
        assert!(error.to_string().contains("managed by cargo-build-musl"));
    }

    #[test]
    fn finds_manifest_path_in_forwarded_arguments() {
        let cli = parse(&["--release", "--manifest-path", "crates/app/Cargo.toml"]).unwrap();
        assert_eq!(
            cli.manifest_path().unwrap(),
            Some(PathBuf::from("crates/app/Cargo.toml"))
        );
    }

    #[test]
    fn environment_options_are_repeatable_and_not_forwarded_to_cargo() {
        let cli = parse(&[
            "--build-musl-env",
            "RUSTFLAGS",
            "--build-musl-env=OPENSSL_STATIC",
            "--release",
        ])
        .unwrap();
        assert_eq!(cli.env, ["RUSTFLAGS", "OPENSSL_STATIC"]);
        assert_eq!(cli.cargo_args, [OsString::from("--release")]);
        for invalid in ["", "A=B", "1INVALID", "CARGO_HOME", "CARGO_TARGET_DIR"] {
            assert!(parse(&["--build-musl-env", invalid]).is_err());
        }
    }
}
