//! Fast local inventory; active device I/O requires an explicitly selected port.
use std::{ffi::OsString, path::Path};

#[derive(Default)]
struct Selection {
    port: Option<OsString>,
    host: Option<OsString>,
}

fn selection(mut args: impl Iterator<Item = OsString>) -> Result<Selection, &'static str> {
    let mut selection = Selection::default();
    while let Some(flag) = args.next() {
        let field = if flag == "--port" {
            &mut selection.port
        } else if flag == "--host" {
            &mut selection.host
        } else {
            return Err("expected inspect [--port PORT] [--host NAME]");
        };
        if field.is_some() {
            return Err("inspection selection repeated");
        }
        let value = args.next().ok_or("missing inspection selection")?;
        if value.is_empty() || value.to_string_lossy().starts_with("--") {
            return Err("missing inspection selection");
        }
        *field = Some(value);
    }
    Ok(selection)
}

pub fn run(
    args: impl Iterator<Item = OsString>,
    directory: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let selected = selection(args)?;
    println!("Coaptic inspection — {}", std::env::consts::OS);
    println!("Credential storage: {}", directory.display());
    println!(
        "PIV approval: {}; Windows WebAuthn: {}",
        if cfg!(feature = "development-piv") {
            "available"
        } else {
            "not built"
        },
        if cfg!(all(windows, feature = "webauthn")) {
            "available"
        } else {
            "not available in this build"
        }
    );
    for (stem, label) in [
        ("piv-touch", "Touch-required credential"),
        ("piv-development", "Touchless development credential"),
        ("operator", "WebAuthn credential"),
    ] {
        println!(
            "{label}: {}",
            if directory.join(format!("{stem}.cbor")).is_file() {
                "record present (not a live key test)"
            } else {
                "not registered"
            }
        );
    }
    match serialport::available_ports() {
        Ok(ports) if ports.is_empty() => println!("Serial ports: none detected"),
        Ok(ports) => {
            for port in ports {
                println!("Serial port: {}", port.port_name);
            }
        }
        Err(error) => println!("Serial port inventory unavailable: {error}"),
    }
    match std::fs::read_dir(directory.join("Devices")) {
        Ok(entries) => {
            for entry in entries.take(129).enumerate() {
                let (index, entry) = entry;
                if index == 128 {
                    println!(
                        "Host inventory truncated at 128 entries; select --host NAME for details."
                    );
                    break;
                }
                match entry {
                    Ok(entry) => match entry.file_type() {
                        Ok(kind) if kind.is_dir() => {
                            println!("Host identity: {}", entry.file_name().to_string_lossy())
                        }
                        Ok(_) => {}
                        Err(error) => println!("Host entry unavailable: {error}"),
                    },
                    Err(error) => println!("Host entry unavailable: {error}"),
                }
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("Host identities: none provisioned")
        }
        Err(error) => println!("Host inventory unavailable: {error}"),
    }
    let mut failed = false;
    if let Some(name) = selected.host
        && let Err(error) = inspect_host(directory, &name)
    {
        println!("Selected host unavailable: {error}");
        failed = true;
    }
    if let Some(port) = selected.port {
        println!(
            "Inspecting selected device (up to 10 seconds): {}",
            port.to_string_lossy()
        );
        if let Err(error) = super::serial_cli::run(
            std::ffi::OsStr::new("serial-info"),
            [port].into_iter(),
            directory,
        ) {
            println!("Selected device unavailable: {error}");
            failed = true;
        }
    } else {
        println!("Use --port PORT for live device state; ports are not opened automatically.");
    }
    if failed {
        return Err(
            "some selected inspection targets were unavailable; other results are shown above"
                .into(),
        );
    }
    Ok(())
}

fn inspect_host(
    directory: &Path,
    name: &std::ffi::OsStr,
) -> Result<(), Box<dyn std::error::Error>> {
    let directory = super::existing_device_directory(directory, name)?;
    let identity = coaptic_enroll::device::restore(&directory)?;
    let peer = identity.peer();
    println!(
        "Host {}: principal {}, credential ID {}",
        name.to_string_lossy(),
        super::hex(peer.principal().fingerprint()),
        peer.kid()
    );
    let store = coaptic_enroll::pairing::PairingFile::open(&directory)?;
    let state = store.restore(&identity)?;
    let record = state.checkpoint();
    println!(
        "Pairing: revision {}, enabled {}",
        record.revision(),
        record.enabled()
    );
    if let Some(policy) = record.policy() {
        println!(
            "Peer {}; owner {}",
            super::hex(policy.peer().principal().fingerprint()),
            super::hex(policy.owner())
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inspection_targets_are_explicit_and_unambiguous() {
        assert!(selection([].into_iter()).unwrap().port.is_none());
        let selected = selection(
            ["--host", "host", "--port", "COM6"]
                .map(OsString::from)
                .into_iter(),
        )
        .unwrap();
        assert_eq!(selected.port.unwrap(), "COM6");
        for args in [
            vec!["--port"],
            vec!["--host", ""],
            vec!["--host", "a", "--host", "b"],
            vec!["--port", "--host"],
            vec!["--all"],
        ] {
            assert!(selection(args.into_iter().map(OsString::from)).is_err());
        }
    }
}
