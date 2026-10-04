# cargo-build-musl

`cargo-build-musl` builds static musl binaries in an official Rust Alpine
container. The MVP supports native-architecture builds on Linux with Docker.

The container is disposable. Cargo downloads are reused from the host's
`$CARGO_HOME`, while musl build artifacts are reused from the project's Cargo
target directory.

## Usage

```console
$ cargo install --path .
$ cargo build-musl --release
```

Cargo build options are forwarded unchanged:

```console
$ cargo build-musl -p server --bin server --all-features --locked
```

Build-musl options must appear before the first forwarded Cargo option:

```console
$ cargo build-musl --build-musl-pull always --release
```

The resulting binary is written to:

```text
target/x86_64-unknown-linux-musl/release/server
```

On an AArch64 host, the target is `aarch64-unknown-linux-musl`.

## Options

```text
--build-musl-image <IMAGE>   Override the Rust Alpine image
--build-musl-pull <POLICY>   Set missing, always, or never
--build-musl-dry-run         Print the Docker command
--build-musl-env <NAME>      Pass a host environment variable (repeatable)
```

The default image is `rust:alpine`.

`--target`, `--target-dir`, and `--message-format` are controlled by
`cargo-build-musl`.

Relative and absolute `--manifest-path` values are mapped into the container,
including when invoked from outside the workspace. Outside callers use the
selected manifest's directory as the container working directory. Other path
options (such as `--config path`) are not rewritten; external path dependencies
are not automatically mounted.

Pass build variables explicitly, before the Cargo options:

```sh
RUSTFLAGS='-C debuginfo=1' OPENSSL_STATIC=1 cargo build-musl \
  --build-musl-env RUSTFLAGS --build-musl-env OPENSSL_STATIC --release
```

The variable must exist on the host (an empty value is allowed). Only its name
appears in the Docker command and dry-run output. `CARGO_HOME` and
`CARGO_TARGET_DIR` remain managed by the tool. Proxy variables are forwarded
automatically; other host variables require `--build-musl-env`.

## Current scope

- Linux hosts only
- x86_64 and AArch64
- Docker, including a rootless Docker or Podman-compatible `docker` command
- Same-architecture builds only
- `cargo build` only

The host Cargo home is mounted read-write at `/cargo-home`. This reuses registry
and Git dependency downloads, and also makes the host Cargo configuration and
credentials visible inside the build container.

Ctrl+C (SIGINT) and SIGTERM cancel the build and force-remove its uniquely named
container, returning exit codes 130 and 143 respectively. Project files and
Cargo caches are retained. SIGKILL cannot be handled.

To run the real-container cancellation regression with a cached `rust:alpine`:

```sh
cargo test --test cancellation -- --ignored --nocapture
```
