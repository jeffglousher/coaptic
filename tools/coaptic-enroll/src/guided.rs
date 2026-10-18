//! Bounded terminal input. Oversized lines are drained before another prompt.
use std::io::{self, BufRead, Write};

fn line<const N: usize>(
    input: &mut impl BufRead,
) -> io::Result<Option<coaptic_console_core::Line<N>>> {
    let mut line = coaptic_console_core::Line::new();
    loop {
        let available = input.fill_buf()?;
        if available.is_empty() {
            if !line.finish() {
                return Ok(None);
            }
            break;
        }
        let byte = available[0];
        input.consume(1);
        if line.push(byte) {
            break;
        }
    }
    if line.text() == Err(coaptic_console_core::InputError::TooLong) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "input exceeds line capacity",
        ));
    }
    Ok(Some(line))
}

fn prompt(
    input: &mut impl BufRead,
    output: &mut impl Write,
    label: &str,
) -> io::Result<Option<String>> {
    loop {
        write!(output, "{label} (:cancel to leave): ")?;
        output.flush()?;
        let value = match line::<256>(input) {
            Ok(Some(line)) => match line.text() {
                Ok(value) => value.trim().to_owned(),
                Err(_) => {
                    writeln!(output, "Enter UTF-8 text.")?;
                    continue;
                }
            },
            Ok(None) => return Ok(None),
            Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
                writeln!(output, "{error}")?;
                continue;
            }
            Err(error) => return Err(error),
        };
        if value == ":cancel" {
            return Ok(None);
        }
        if !value.is_empty() {
            return Ok(Some(value));
        }
    }
}

/// Collect one reviewed operation; no command executes during collection.
pub fn collect(
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<Option<Vec<std::ffi::OsString>>> {
    writeln!(
        output,
        "Coaptic commissioning\n1 Inspect device\n2 Pair\n3 Revoke\n4 Transfer ownership\n5 Recover host pairing\n6 Initialize device identity\n7 Inspect host identity\n8 Initialize host identity\n9 Register touch-required PIV credential\n10 Register touchless development PIV credential\n11 Prepare native credential storage\n12 Connect protected peer\n13 Register Windows WebAuthn operator"
    )?;
    let Some(choice) = prompt(input, output, "Operation")? else {
        return Ok(None);
    };
    let (command, fields): (&str, &[&str]) = match choice.as_str() {
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
        _ => {
            writeln!(output, "Unknown operation; no changes made.")?;
            return Ok(None);
        }
    };
    let mut args = vec![command.into()];
    for field in fields {
        let Some(value) = prompt(input, output, field)? else {
            return Ok(None);
        };
        args.push(std::ffi::OsString::from(value));
    }
    if choice == "12" {
        args.push("--connect".into());
    }
    if matches!(choice.as_str(), "2" | "3" | "4" | "5") {
        let Some(mode) = prompt(
            input,
            output,
            "Approval: 1 Windows verified authenticator, 2 development PIV no-touch, 3 PIV touch required",
        )?
        else {
            return Ok(None);
        };
        match mode.as_str() {
            "1" => {}
            "2" => args.push("--development-no-touch".into()),
            "3" => args.push("--piv-touch".into()),
            _ => {
                writeln!(output, "Unknown approval mode; no changes made.")?;
                return Ok(None);
            }
        }
    }
    writeln!(output, "Selected operation: {args:?}")?;
    let Some(confirm) = prompt(input, output, "Type 'continue' to execute")? else {
        return Ok(None);
    };
    if confirm != "continue" {
        writeln!(output, "Cancelled; no changes made.")?;
        return Ok(None);
    }
    Ok(Some(args))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overflow_drains_the_entire_line() {
        let mut input = io::Cursor::new(b"oversized\nnext\n");
        assert!(
            matches!(line::<4>(&mut input), Err(error) if error.kind() == io::ErrorKind::InvalidInput)
        );
        let line = line::<4>(&mut input).unwrap().unwrap();
        assert_eq!(line.text(), Ok("next"));
    }
    #[test]
    fn cancellation_and_eof_never_execute() {
        for text in ["2\n:cancel\n", "2\nCOM6\n", "1\nCOM6\nno\n"] {
            assert!(
                collect(&mut io::Cursor::new(text), &mut Vec::new())
                    .unwrap()
                    .is_none()
            );
        }
    }
    #[test]
    fn explicit_touchless_selection_is_preserved() {
        let result = collect(
            &mut io::Cursor::new("2\nCOM6\nhost\n192.0.2.1:5683\n2\ncontinue\n"),
            &mut Vec::new(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(result.last().unwrap(), "--development-no-touch");
    }

    #[test]
    fn touch_required_selection_is_not_touchless() {
        let result = collect(
            &mut io::Cursor::new("2\nCOM6\nhost\n192.0.2.1:5683\n3\ncontinue\n"),
            &mut Vec::new(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(result.last().unwrap(), "--piv-touch");
        assert!(!result.iter().any(|arg| arg == "--development-no-touch"));
    }
}
