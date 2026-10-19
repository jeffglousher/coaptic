//! Shared commissioning operations and prompts; no transport or allocation dependency.

pub struct Operation {
    pub command: &'static str,
    pub description: &'static str,
    pub fields: &'static [&'static str],
    pub approval: bool,
    pub connect: bool,
    pub permissions: bool,
}

pub fn operation(choice: &str) -> Option<Operation> {
    let (command, fields): (&str, &[&str]) = match choice {
        "1" => ("inspect", &[]),
        "2" => (
            "serial-pair",
            &[
                "Serial port",
                "Existing host identity name",
                "Host IPv4 address:port",
            ],
        ),
        "3" => (
            "serial-revoke",
            &["Serial port", "Existing host identity name"],
        ),
        "4" => (
            "serial-transfer",
            &[
                "Serial port",
                "New unpaired host identity name",
                "Host IPv4 address:port",
                "Verified new owner commitment (hex)",
            ],
        ),
        "5" => (
            "serial-recover",
            &["Serial port", "Unpaired host identity name"],
        ),
        "6" => ("serial-init", &["Serial port", "Credential ID (0-255)"]),
        "7" => ("device-info", &["Host identity name"]),
        "8" => (
            "device-init",
            &["New host identity name", "Credential ID (0-255)"],
        ),
        "9" => (
            "piv-touch-register",
            &[
                "YubiKey serial",
                "Existing PIV slot (hex)",
                "Public key PEM path",
            ],
        ),
        "10" => (
            "development-piv-register",
            &[
                "YubiKey serial",
                "Existing PIV slot (hex)",
                "Public key PEM path",
            ],
        ),
        "11" => ("custody-init", &[]),
        "12" => (
            "peer",
            &[
                "Host identity name",
                "Local address:port",
                "Peer address:port",
            ],
        ),
        "13" => ("operator-init", &[]),
        "14" => ("piv-touch-test", &[]),
        "15" => ("development-piv-test", &[]),
        "16" => ("serial-restart", &["Serial port"]),
        _ => return None,
    };
    Some(Operation {
        command,
        description: match choice {
            "1" => "Inspect local credentials, identities and selected device/host",
            "2" => "Pair device with this host",
            "3" => "Revoke the device pairing",
            "4" => "Transfer device ownership to a verified new owner",
            "5" => "Recover this host's existing device pairing",
            "6" => "Initialize an empty device identity",
            "7" => "Inspect host identity",
            "8" => "Create a host identity",
            "9" => "Register an existing touch-required YubiKey credential",
            "10" => "Register an existing touchless development YubiKey credential",
            "11" => "Prepare native credential storage",
            "12" => "Connect to a protected peer",
            "13" => "Register a Windows verified authenticator",
            "14" => "Test the touch-required credential",
            "15" => "Test the touchless development credential",
            "16" => "Restart device, retaining identity and pairing",
            _ => unreachable!(),
        },
        fields,
        approval: matches!(choice, "2" | "3" | "4" | "5"),
        connect: choice == "12",
        permissions: matches!(choice, "2" | "4"),
    })
}

/// A small menu group; operation IDs refer to [`operation`].
pub struct Group {
    pub title: &'static str,
    pub operations: &'static [&'static str],
}

/// Inspect is a direct overview; remaining entries collect one focused operation.
pub fn group(choice: &str) -> Option<Group> {
    let (title, operations): (&str, &[&str]) = match choice {
        "1" => ("Inspect", &["1"]),
        "2" => ("Set up", &["11", "8", "6"]),
        "3" => ("Pair / connect", &["2", "12", "5"]),
        "4" => ("Credentials", &["9", "10", "14", "15", "13"]),
        "5" => ("Maintenance", &["16", "3", "4"]),
        _ => return None,
    };
    Some(Group { title, operations })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_authorization_changes_request_approval() {
        assert!(operation("2").unwrap().approval);
        assert!(operation("3").unwrap().approval);
        assert!(operation("4").unwrap().approval);
        assert!(operation("5").unwrap().approval);
        assert!(!operation("1").unwrap().approval);
        assert!(operation("unknown").is_none());
    }
}
