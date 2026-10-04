use std::fs;
use std::process::Command;

// Invoke the CLI from a different directory and use a fake container engine to
// check the actual argv without requiring Docker in the test environment.
#[test]
#[cfg(unix)]
fn outside_workspace_manifest_paths_and_environment_reach_container_command() {
    use std::os::unix::fs::PermissionsExt;
    let root = std::env::temp_dir().join(format!("build-musl-paths-{}", std::process::id()));
    fs::create_dir_all(root.join("project with spaces/src")).unwrap();
    fs::create_dir_all(root.join("bin")).unwrap();
    fs::write(
        root.join("project with spaces/Cargo.toml"),
        "[package]\nname='path-fixture'\nversion='0.1.0'\nedition='2024'\n[workspace]\n",
    )
    .unwrap();
    fs::write(
        root.join("project with spaces/src/main.rs"),
        "fn main() {}\n",
    )
    .unwrap();
    let docker = root.join("bin/docker");
    fs::write(
        &docker,
        "#!/bin/sh\nif [ \"$1\" = info ]; then printf 'true\\n'; else exit 99; fi\n",
    )
    .unwrap();
    fs::set_permissions(&docker, fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::join_paths(
        std::iter::once(root.join("bin"))
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let manifest = root.join("project with spaces/Cargo.toml");
    for args in [
        vec![
            "--manifest-path".to_owned(),
            manifest.to_str().unwrap().to_owned(),
        ],
        vec!["--manifest-path=project with spaces/Cargo.toml".to_owned()],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_cargo-build-musl"))
            .current_dir(&root)
            .env("PATH", &path)
            .env("CARGO_HOME", root.join("cargo-home"))
            .env("MUSL_TEST_VARIABLE", "value-that-must-not-be-printed")
            .args([
                "build-musl",
                "--build-musl-dry-run",
                "--build-musl-env",
                "MUSL_TEST_VARIABLE",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = String::from_utf8(output.stdout).unwrap();
        assert!(
            output.contains("\"--manifest-path\" \"/workspace/Cargo.toml\""),
            "{output}"
        );
        assert!(output.contains("\"--env\" \"MUSL_TEST_VARIABLE\""));
        assert!(!output.contains("value-that-must-not-be-printed"));
    }
    fs::remove_dir_all(&root).unwrap();
}
