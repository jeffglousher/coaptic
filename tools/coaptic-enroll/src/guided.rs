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

/// Availability of platform-specific operator registration and approval.
#[derive(Clone, Copy)]
pub struct Capabilities {
    pub webauthn: bool,
    pub piv: bool,
}

fn available(id: &str, capabilities: Capabilities) -> bool {
    match id {
        "13" => capabilities.webauthn,
        "9" | "10" | "14" | "15" => capabilities.piv,
        "2" | "3" | "4" | "5" | "18" | "19" | "20" => capabilities.piv || capabilities.webauthn,
        _ => true,
    }
}

enum Input {
    Value(String),
    Back,
    Cancel,
}
fn prompt(
    input: &mut impl BufRead,
    output: &mut impl Write,
    label: &str,
    optional: bool,
) -> io::Result<Input> {
    loop {
        write!(output, "{label}: ")?;
        output.flush()?;
        let value = match line::<256>(input) {
            Ok(Some(line)) => match line.text() {
                Ok(value) => value.trim().to_owned(),
                Err(_) => {
                    writeln!(output, "Enter UTF-8 text.")?;
                    continue;
                }
            },
            Ok(None) => return Ok(Input::Cancel),
            Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
                writeln!(output, "{error}")?;
                continue;
            }
            Err(error) => return Err(error),
        };
        match value.as_str() {
            ":cancel" => return Ok(Input::Cancel),
            ":back" => return Ok(Input::Back),
            "" if !optional => writeln!(output, "Enter a value, :back or :cancel.")?,
            _ => return Ok(Input::Value(value)),
        }
    }
}

enum Selection {
    Ready(Vec<std::ffi::OsString>),
    Back,
    Cancel,
}
macro_rules! answer {
    ($input:expr, $output:expr, $label:expr, $optional:expr) => {
        match prompt($input, $output, $label, $optional)? {
            Input::Value(value) => value,
            Input::Back => return Ok(Selection::Back),
            Input::Cancel => return Ok(Selection::Cancel),
        }
    };
}

fn collect_operation(
    operation: coaptic_console_core::workflow::Operation,
    capabilities: Capabilities,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> io::Result<Selection> {
    let mut args = vec![operation.command.into()];
    let mut review = Vec::new();
    if operation.command == "inspect" {
        for (label, flag) in [
            ("Serial port (blank skips device query)", "--port"),
            ("Host identity (blank skips targeted host query)", "--host"),
        ] {
            let value = answer!(input, output, label, true);
            if !value.is_empty() {
                review.push((label, value.clone()));
                args.push(flag.into());
                args.push(value.into());
            }
        }
    }
    for field in operation.fields {
        let value = answer!(input, output, field, false);
        review.push((*field, value.clone()));
        args.push(value.into());
    }
    if operation.connect {
        args.push("--connect".into());
    }
    if operation.approval {
        loop {
            if capabilities.piv {
                writeln!(
                    output,
                    "Approval: 1 Touch-required YubiKey, 2 Touchless development YubiKey"
                )?;
            }
            if capabilities.webauthn {
                writeln!(output, "          3 Windows verified authenticator")?;
            }
            let choice = answer!(input, output, "Approval choice", false);
            let label = match choice.as_str() {
                "1" if capabilities.piv => {
                    args.push("--piv-touch".into());
                    "Touch-required YubiKey"
                }
                "2" if capabilities.piv => {
                    args.push("--development-no-touch".into());
                    "Touchless development YubiKey"
                }
                "3" if capabilities.webauthn => "Windows verified authenticator",
                _ => {
                    writeln!(output, "Choose an available approval mode.")?;
                    continue;
                }
            };
            review.push(("Approval", label.to_owned()));
            break;
        }
    }
    if operation.permissions {
        loop {
            let choice = answer!(
                input,
                output,
                "Access: 1 Read-only, 2 Read/write (echo and durable telemetry)",
                false
            );
            let label = match choice.as_str() {
                "1" => {
                    args.push("--read-only".into());
                    "Read-only"
                }
                "2" => "Read/write",
                _ => {
                    writeln!(output, "Choose 1 or 2.")?;
                    continue;
                }
            };
            review.push(("Access", label.to_owned()));
            break;
        }
    }
    writeln!(output, "\nReview: {}", operation.description)?;
    for (label, value) in review {
        writeln!(output, "  {label}: {value}")?;
    }
    loop {
        let confirm = answer!(
            input,
            output,
            "Type 'continue' to execute, :back to change selection",
            false
        );
        if confirm == "continue" {
            return Ok(Selection::Ready(args));
        }
        writeln!(output, "Enter 'continue', :back or :cancel.")?;
    }
}

/// Collect one reviewed operation; no command executes during collection.
pub fn collect(
    input: &mut impl BufRead,
    output: &mut impl Write,
    capabilities: Capabilities,
) -> io::Result<Option<Vec<std::ffi::OsString>>> {
    writeln!(
        output,
        "Coaptic commissioning - :back returns, :cancel exits."
    )?;
    loop {
        writeln!(
            output,
            "\n1 Inspect - local overview, optional device/host query\n2 Set up - storage and identities\n3 Pair / connect - establish or recover communication"
        )?;
        if capabilities.piv || capabilities.webauthn {
            writeln!(output, "4 Credentials - register or test approval keys")?;
        }
        writeln!(
            output,
            "5 Maintenance - restart, revoke or transfer ownership"
        )?;
        let choice = match prompt(input, output, "Group", false)? {
            Input::Value(value) => value,
            Input::Back => continue,
            Input::Cancel => return Ok(None),
        };
        let Some(group) = coaptic_console_core::workflow::group(&choice) else {
            writeln!(output, "Choose a group from 1 to 5.")?;
            continue;
        };
        let operations: Vec<_> = group
            .operations
            .iter()
            .copied()
            .filter(|id| available(id, capabilities))
            .collect();
        if operations.is_empty() {
            writeln!(
                output,
                "This group has no available operations in this build."
            )?;
            continue;
        }
        loop {
            let operation = if choice == "1" {
                coaptic_console_core::workflow::operation("1").unwrap()
            } else {
                writeln!(output, "\n{}", group.title)?;
                for (index, id) in operations.iter().enumerate() {
                    let operation = coaptic_console_core::workflow::operation(id).unwrap();
                    writeln!(output, "{} {}", index + 1, operation.description)?;
                }
                let selected = match prompt(input, output, "Operation (:back for groups)", false)? {
                    Input::Value(value) => value,
                    Input::Back => break,
                    Input::Cancel => return Ok(None),
                };
                let id = selected
                    .parse::<usize>()
                    .ok()
                    .and_then(|n| n.checked_sub(1))
                    .and_then(|index| operations.get(index));
                let Some(id) = id else {
                    writeln!(output, "Choose an available operation.")?;
                    continue;
                };
                coaptic_console_core::workflow::operation(id).unwrap()
            };
            match collect_operation(operation, capabilities, input, output)? {
                Selection::Ready(args) => return Ok(Some(args)),
                Selection::Cancel => return Ok(None),
                Selection::Back if choice == "1" => break,
                Selection::Back => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn collect_text(text: &str) -> (Option<Vec<std::ffi::OsString>>, String) {
        let mut output = Vec::new();
        let result = collect(
            &mut io::Cursor::new(text),
            &mut output,
            Capabilities {
                webauthn: false,
                piv: true,
            },
        )
        .unwrap();
        (result, String::from_utf8(output).unwrap())
    }
    #[test]
    fn overflow_drains_the_entire_line() {
        let mut input = io::Cursor::new(b"oversized\nnext\n");
        assert!(
            matches!(line::<4>(&mut input), Err(error) if error.kind() == io::ErrorKind::InvalidInput)
        );
        assert_eq!(line::<4>(&mut input).unwrap().unwrap().text(), Ok("next"));
    }
    #[test]
    fn cancellation_and_eof_never_execute() {
        for text in [":cancel\n", "3\n1\nCOM6\n", "1\n\n\n:cancel\n"] {
            assert!(collect_text(text).0.is_none());
        }
    }
    #[test]
    fn inspect_blank_and_targeted_inputs() {
        assert_eq!(collect_text("1\n\n\ncontinue\n").0.unwrap(), ["inspect"]);
        assert_eq!(
            collect_text("1\nCOM6\nhost\ncontinue\n").0.unwrap(),
            ["inspect", "--port", "COM6", "--host", "host"]
        );
    }
    #[test]
    fn back_and_invalid_choices_preserve_navigation() {
        let (args, output) = collect_text("invalid\n2\n99\n1\n:back\n:back\n1\n\n\ncontinue\n");
        assert_eq!(args.unwrap(), ["inspect"]);
        assert!(output.contains("Choose a group"));
        assert!(output.contains("Choose an available operation"));
    }
    #[test]
    fn approval_and_access_retry_without_silent_fallback() {
        let (args, output) =
            collect_text("3\n1\nCOM6\nhost\n192.0.2.1:5683\n3\n2\ninvalid\n1\ncontinue\n");
        let args = args.unwrap();
        assert!(args.iter().any(|arg| arg == "--development-no-touch"));
        assert!(args.iter().any(|arg| arg == "--read-only"));
        assert!(!output.contains("Windows verified authenticator"));
        assert!(output.contains("Approval: Touchless development YubiKey"));
    }
    #[test]
    fn touch_required_profile_and_hidden_registration() {
        let (args, output) =
            collect_text("4\n5\n:back\n3\n1\nCOM6\nhost\n192.0.2.1:5683\n1\n2\ncontinue\n");
        let args = args.unwrap();
        assert!(args.iter().any(|arg| arg == "--piv-touch"));
        assert!(!args.iter().any(|arg| arg == "--development-no-touch"));
        assert!(!output.contains("Windows verified authenticator"));
    }

    #[test]
    fn webauthn_only_build_omits_piv_options() {
        let mut output = Vec::new();
        let args = collect(
            &mut io::Cursor::new("4\n1\ncontinue\n"),
            &mut output,
            Capabilities {
                webauthn: true,
                piv: false,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(args, ["operator-init"]);
        let output = String::from_utf8(output).unwrap();
        assert!(!output.contains("YubiKey"));
    }
}
