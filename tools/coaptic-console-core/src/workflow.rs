//! Shared commissioning operations and prompts; no transport or allocation dependency.

pub struct Operation {
    pub command: &'static str,
    pub fields: &'static [&'static str],
    pub approval: bool,
    pub connect: bool,
}

pub fn operation(choice: &str) -> Option<Operation> {
    let (command, fields): (&str, &[&str]) = match choice {
        "1" => ("serial-info", &["Serial port"]),
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
        _ => return None,
    };
    Some(Operation {
        command,
        fields,
        approval: matches!(choice, "2" | "3" | "4" | "5"),
        connect: choice == "12",
    })
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
