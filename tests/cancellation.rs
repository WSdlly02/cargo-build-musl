#![cfg(unix)]

use std::os::unix::process::CommandExt;
use std::{
    fs,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

// Real engine regression: a build script deliberately keeps compiling until
// cancellation. Run explicitly with a cached rust:alpine image.
#[test]
#[ignore = "requires Docker or Podman and a cached rust:alpine image"]
fn interrupts_remove_the_running_container() {
    for (signal, code) in [("INT", 130), ("TERM", 143)] {
        let root =
            std::env::temp_dir().join(format!("musl-cancel-{}-{signal}", std::process::id()));
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='cancel-test'\nversion='0.1.0'\nedition='2024'\n[workspace]\n",
        )
        .unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(root.join("build.rs"), "fn main() { std::fs::write(\"started\", \"ready\").unwrap(); loop { std::thread::sleep(std::time::Duration::from_secs(1)); } }\n").unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_cargo-build-musl"))
            .args(["--build-musl-pull", "never"])
            .current_dir(&root)
            .process_group(0)
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while !root.join("started").exists() && Instant::now() < deadline {
            assert!(
                child.try_wait().unwrap().is_none(),
                "build exited before marker"
            );
            thread::sleep(Duration::from_millis(100));
        }
        // SIGINT to the foreground process group models Ctrl+C; SIGTERM is
        // delivered only to the wrapper to ensure cleanup is wrapper-owned.
        let target = if signal == "INT" {
            format!("-{}", child.id())
        } else {
            child.id().to_string()
        };
        assert!(
            Command::new("kill")
                .args([&format!("-{signal}"), "--", &target])
                .status()
                .unwrap()
                .success()
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "cancellation timed out");
            thread::sleep(Duration::from_millis(100));
        };
        assert_eq!(status.code(), Some(code));
        let output = Command::new("docker")
            .args([
                "ps",
                "-aq",
                "--filter",
                &format!("name=cargo-build-musl-{}-", child.id()),
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stdout.is_empty(), "container survived cancellation");
        assert!(
            root.join("started").exists(),
            "did not reach running build script"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
