use std::process::Command;

#[test]
fn help_does_not_open_credential_storage() {
    // A relative enrollment directory is refused by ordinary commands. Help
    // must remain available before custody, storage or device access begins.
    for arguments in [
        vec!["--help"],
        vec!["-h"],
        vec!["inspect", "--help"],
        vec!["inspect", "-h"],
        vec!["peer", "--help"],
        vec!["peer", "-h"],
        vec!["serial-reconcile", "--help"],
        vec!["serial-reconcile", "-h"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_coaptic-enroll"))
            .env("COAPTIC_ENROLLMENT_DIR", "relative-help-must-not-open")
            .args(&arguments)
            .output()
            .unwrap();
        assert!(output.status.success(), "{arguments:?}: {output:?}");
        assert!(!output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
    for arguments in [vec!["unknown", "--help"], vec!["--help", "unexpected"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_coaptic-enroll"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(!output.status.success());
    }
    let peer = Command::new(env!("CARGO_BIN_EXE_coaptic-enroll"))
        .args(["peer", "--help"])
        .output()
        .unwrap();
    let peer = String::from_utf8(peer.stdout).unwrap();
    assert!(peer.contains("Qualification-only fault injection"));
    assert!(peer.contains("--qualify-device-restart"));
    assert!(peer.contains("host PID"));
    assert!(peer.contains("fresh session alone does not prove"));
    assert!(peer.contains("responding host needs OS firewall permission"));
}
