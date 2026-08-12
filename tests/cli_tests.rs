use std::process::Command;

const RUNCOVE_BIN: &str = env!("CARGO_BIN_EXE_runcove");
const PORTPEEK_BIN: &str = env!("CARGO_BIN_EXE_portpeek");

#[test]
fn test_help_flag() {
    let output = Command::new(RUNCOVE_BIN)
        .arg("--help")
        .output()
        .expect("Failed to run portpeek --help");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("runcove"));
    assert!(stdout.contains("Local dev services, under control"));
}

#[test]
fn test_version_flag() {
    let output = Command::new(RUNCOVE_BIN)
        .arg("--version")
        .output()
        .expect("Failed to run portpeek --version");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("runcove"));
}

#[test]
fn test_json_output() {
    let output = Command::new(RUNCOVE_BIN)
        .args(["--json", "--no-color"])
        .output()
        .expect("Failed to run portpeek --json");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should be valid JSON (even if empty array)
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(stdout.trim());
    assert!(parsed.is_ok(), "Output should be valid JSON: {}", stdout);
}

#[test]
fn test_no_color_flag() {
    let output = Command::new(RUNCOVE_BIN)
        .args(["--no-color"])
        .output()
        .expect("Failed to run portpeek --no-color");

    assert!(output.status.success());
}

#[test]
fn test_specific_port() {
    // Querying a port that likely isn't in use should succeed (empty result)
    let output = Command::new(RUNCOVE_BIN)
        .args(["--no-color", "59999"])
        .output()
        .expect("Failed to run portpeek with port filter");

    assert!(output.status.success());
}

#[test]
fn test_kill_rejects_zero_port_without_scanning() {
    let output = Command::new(RUNCOVE_BIN)
        .args(["kill", "0", "--force"])
        .output()
        .expect("Failed to run runcove kill");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("Port must be between 1 and 65535"));
}

#[test]
fn test_range_format() {
    // Invalid range format should produce a warning
    let output = Command::new(RUNCOVE_BIN)
        .args(["--no-color", "--range", "invalid"])
        .output()
        .expect("Failed to run portpeek --range");

    // Should still succeed (just ignore bad range with a warning)
    assert!(output.status.success());
}

#[test]
fn test_legacy_portpeek_alias() {
    let output = Command::new(PORTPEEK_BIN)
        .arg("--version")
        .output()
        .expect("Failed to run legacy portpeek alias");

    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("runcove"));
}

#[test]
fn test_legacy_portpeek_argument_and_exit_code_compatibility() {
    let cases: &[&[&str]] = &[
        &["--no-color", "59999"],
        &["--json", "--no-color", "59999"],
        &["--no-color", "--range", "59999-60000"],
        &["kill", "0", "--force"],
    ];

    for args in cases {
        let primary = Command::new(RUNCOVE_BIN)
            .args(*args)
            .output()
            .expect("Failed to run runcove compatibility case");
        let legacy = Command::new(PORTPEEK_BIN)
            .args(*args)
            .output()
            .expect("Failed to run portpeek compatibility case");

        assert_eq!(
            primary.status.code(),
            legacy.status.code(),
            "exit code differs for arguments {args:?}"
        );
        if args.contains(&"--json") {
            assert!(serde_json::from_slice::<serde_json::Value>(&primary.stdout).is_ok());
            assert!(serde_json::from_slice::<serde_json::Value>(&legacy.stdout).is_ok());
        }
    }
}

#[test]
fn test_watch_rejects_zero_interval() {
    for binary in [RUNCOVE_BIN, PORTPEEK_BIN] {
        let output = Command::new(binary)
            .args(["--watch", "-w", "0"])
            .output()
            .expect("Failed to validate watch interval");
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr)
            .contains("Refresh interval must be at least 1 second"));
    }
}
