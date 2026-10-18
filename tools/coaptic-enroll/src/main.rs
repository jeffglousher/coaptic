//! Trusted local Windows operator registration and device-key provisioning.
//! PINs are handled by Windows Security, never by this process or its logs.
#[cfg(windows)]
use coaptic_enroll::{
    Request, begin_registration, origin,
    store::{self, OperatorStore},
};
#[cfg(windows)]
use std::path::PathBuf;

#[cfg(all(windows, feature = "development-piv"))]
mod piv_cli;

#[cfg(windows)]
mod pairing_cli;

#[cfg(windows)]
mod serial_cli;

fn main() {
    if let Err(error) = run() {
        eprintln!("coaptic-enroll: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    Err("this native hardware client requires Windows; the verifier is portable Rust".into())
}

#[cfg(windows)]
fn run() -> Result<(), Box<dyn std::error::Error>> {
    use webauthn_authenticator_rs::{WebauthnAuthenticator, win10::Win10};
    let mut args = std::env::args_os().skip(1);
    let command = args
        .next()
        .ok_or("expected operator-init or operator-check REQUEST.cbor")?;
    if command == "--help" {
        println!(
            "coaptic-enroll operator-init | operator-check REQUEST.cbor | device-init NAME KID | device-info NAME\npair DEVICE PEER [--read-only] [--development-no-touch] | revoke DEVICE [--development-no-touch]\ntransfer DEVICE NEW_PEER VERIFIED_NEW_OWNER_HEX [--read-only] [--development-no-touch]\npairing-init DEVICE | pairing-info DEVICE | peer DEVICE LOCAL_ADDRESS PEER_ADDRESS [--connect | --probe | --probe-shared | --probe-echo | --probe-denied]"
        );
        #[cfg(feature = "development-piv")]
        println!(
            "development-piv-register SERIAL SLOT PUBLIC.pem | development-piv-check REQUEST.cbor"
        );
        println!(
            "serial-info PORT | serial-init PORT KID | serial-restart PORT\nserial-pair PORT HOST_NAME HOST_ADDRESS [--read-only] [--development-no-touch]\nserial-transfer PORT NEW_UNPAIRED_HOST HOST_ADDRESS VERIFIED_NEW_OWNER_HEX [--read-only] [--development-no-touch]\nserial-recover PORT UNPAIRED_HOST [--development-no-touch]\nserial-revoke PORT HOST_NAME [--development-no-touch]"
        );
        return Ok(());
    }
    #[cfg(feature = "development-piv")]
    let development = command == "development-piv-register" || command == "development-piv-check";
    #[cfg(not(feature = "development-piv"))]
    let development = false;
    if ![
        "operator-init",
        "operator-check",
        "device-init",
        "device-info",
        "pair",
        "revoke",
        "transfer",
        "pairing-init",
        "pairing-info",
        "peer",
        "serial-info",
        "serial-init",
        "serial-restart",
        "serial-pair",
        "serial-revoke",
        "serial-transfer",
        "serial-recover",
    ]
    .iter()
    .any(|value| command == *value)
        && !development
    {
        return Err("expected operator-init or operator-check REQUEST.cbor".into());
    }
    let directory = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or("LOCALAPPDATA unavailable")?
        .join("Coaptic")
        .join("Enrollment");
    private_directory(&directory)?;
    if [
        "serial-info",
        "serial-init",
        "serial-restart",
        "serial-pair",
        "serial-revoke",
        "serial-transfer",
        "serial-recover",
    ]
    .iter()
    .any(|value| command == *value)
    {
        return serial_cli::run(&command, args, &directory);
    }
    #[cfg(feature = "development-piv")]
    if development {
        return piv_cli::run(&command, args, &directory);
    }
    if [
        "pair",
        "revoke",
        "transfer",
        "pairing-init",
        "pairing-info",
        "peer",
    ]
    .iter()
    .any(|value| command == *value)
    {
        return pairing_cli::run(&command, args, &directory);
    }
    if command == "device-init" || command == "device-info" {
        let name = args.next().ok_or("missing device name")?;
        let name = name.to_str().ok_or("invalid device name")?;
        if name.is_empty()
            || name.len() > 32
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(
                "device name must be 1..32 ASCII letters, digits, hyphens or underscores".into(),
            );
        }
        let device_directory = directory.join("Devices").join(name);
        let identity = if command == "device-init" {
            let kid = args.next().ok_or("missing credential ID")?;
            let kid: u8 = kid.to_str().ok_or("invalid credential ID")?.parse()?;
            if args.next().is_some() {
                return Err("unexpected arguments".into());
            }
            if device_directory.try_exists()? {
                return Err(
                    "existing device directory requires recovery, not initialization".into(),
                );
            }
            private_directory(&device_directory)?;
            let identity = coaptic_enroll::device::provision(&device_directory, kid)?;
            coaptic_enroll::pairing::PairingFile::open(&device_directory)?.initialize(&identity)?;
            identity
        } else {
            if args.next().is_some() {
                return Err("unexpected arguments".into());
            }
            // Never create a missing device directory during recovery.
            if !device_directory.is_dir() {
                return Err("device is not provisioned".into());
            }
            private_directory(&device_directory)?;
            coaptic_enroll::device::restore(&device_directory)?
        };
        let peer = identity.peer();
        println!("Device {name}: {}", hex(peer.principal().fingerprint()));
        println!(
            "Public key {} credential ID {}",
            hex(&peer.public_key()),
            peer.kid()
        );
        println!("Device identity ready. Peer authorization requires commissioning.");
        return Ok(());
    }
    // Ordinary device-key recovery does not wait on an operator ceremony lock.
    let operator_store = OperatorStore::open(&directory)?;
    let mut client = WebauthnAuthenticator::new(Win10::default());
    if command == "operator-init" {
        if args.next().is_some() || operator_store.exists() {
            return Err("operator already initialized or invalid arguments".into());
        }
        let mut owner = [0; 32];
        getrandom::fill(&mut owner)?;
        let (challenge, pending) = begin_registration(owner)?;
        println!(
            "Register a Coaptic operator key in Windows Security. Enter any PIN only in that dialog."
        );
        let proof = client.do_registration(origin(), challenge)?;
        let operator = pending.finish(&proof)?;
        operator_store.initialize(&operator)?;
        println!(
            "Operator registered. Public owner commitment: {}",
            hex(operator.owner())
        );
    } else if command == "operator-check" {
        let request_path = args
            .next()
            .map(PathBuf::from)
            .ok_or("missing REQUEST.cbor")?;
        if args.next().is_some() {
            return Err("unexpected arguments".into());
        }
        let request: Request = store::read(&request_path)?;
        let commitment = request.commitment()?;
        println!("Review commissioning transaction {}", hex(&commitment));
        println!(
            "Device {}",
            hex(request.device.peer()?.principal().fingerprint())
        );
        println!(
            "Peer   {}",
            hex(request.peer.peer()?.principal().fingerprint())
        );
        println!("Owner  {}", hex(&request.owner));
        println!("Policy {}", hex(&request.permissions));
        let operator = operator_store.load()?;
        let (challenge, pending) = operator.begin(request)?;
        println!("Approve this exact transaction in Windows Security using your registered key.");
        let proof = client.do_authentication(origin(), challenge)?;
        let approved = pending.finish(&proof, |old, new| {
            operator_store.commit(old, new).map_err(|_| ())
        })?;
        assert_eq!(approved.commitment()?, commitment);
        println!(
            "Operator proof verified and counter persisted. Provider check complete; no device trust was changed."
        );
    } else {
        return Err("expected operator-init or operator-check REQUEST.cbor".into());
    }
    Ok(())
}

#[cfg(windows)]
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(windows)]
fn device_directory(
    directory: &std::path::Path,
    name: &std::ffi::OsStr,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let name = name.to_str().ok_or("invalid device name")?;
    if name.is_empty()
        || name.len() > 32
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(
            "device name must be 1..32 ASCII letters, digits, hyphens or underscores".into(),
        );
    }
    let result = directory.join("Devices").join(name);
    if !result.is_dir() {
        return Err("device is not provisioned".into());
    }
    private_directory(&result)?;
    Ok(result)
}

#[cfg(windows)]
fn private_directory(directory: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use std::process::Command;
    use windows::Win32::{
        Foundation::PSID,
        Security::{
            Authorization::{
                ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
                SE_FILE_OBJECT, SetNamedSecurityInfoW,
            },
            DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl,
            PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        },
        System::Memory::LocalFree,
    };
    use windows::core::PCWSTR;

    // Check every ancestor before creating or changing anything beneath it.
    // Windows junctions are reparse points even when is_symlink() is false.
    for ancestor in directory.ancestors() {
        match ancestor.symlink_metadata() {
            Ok(metadata) if metadata.file_attributes() & 0x400 != 0 => {
                return Err("operator directory ancestors must not be reparse points".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    std::fs::create_dir_all(directory)?;
    let system = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .ok_or("SystemRoot unavailable")?
        .join("System32");
    let output = Command::new(system.join("whoami.exe"))
        .args(["/user", "/fo", "csv", "/nh"])
        .output()?;
    if !output.status.success() {
        return Err("cannot identify authenticated OS administrator".into());
    }
    // Only the SID is ASCII; the username can use the system OEM codepage.
    let sid = std::str::from_utf8(
        output
            .stdout
            .rsplit(|byte| *byte == b',')
            .next()
            .ok_or("missing OS user SID")?,
    )?
    .trim()
    .trim_matches('"');
    if !sid.starts_with("S-1-")
        || !sid
            .chars()
            .all(|c| c.is_ascii_digit() || c == 'S' || c == '-')
    {
        return Err("invalid OS user SID".into());
    }
    // Replace the complete DACL: only this OS user and SYSTEM have access.
    let sddl: Vec<u16> = format!("D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)")
        .encode_utf16()
        .chain([0])
        .collect();
    struct Descriptor(PSECURITY_DESCRIPTOR);
    impl Drop for Descriptor {
        fn drop(&mut self) {
            // SAFETY: ConvertStringSecurityDescriptor allocates with LocalAlloc.
            unsafe {
                LocalFree(self.0.0 as isize);
            }
        }
    }
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: NUL-terminated SDDL and valid output pointer; allocation is owned below.
    if !unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(sddl.as_ptr()),
            SDDL_REVISION_1,
            &mut descriptor,
            None,
        )
    }
    .as_bool()
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let descriptor = Descriptor(descriptor);
    let (mut present, mut defaulted, mut dacl) = (0, 0, std::ptr::null_mut());
    // SAFETY: descriptor remains alive and output pointers are valid.
    if !unsafe { GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut dacl, &mut defaulted) }
        .as_bool()
        || present == 0
        || dacl.is_null()
    {
        return Err("private operator DACL unavailable".into());
    }
    let mut targets = vec![directory.to_path_buf()];
    for name in [
        "operator.lock",
        "operator.cbor",
        "device.cbor",
        "piv-development.lock",
        "piv-development.cbor",
        "pairing.lock",
        "pairing.cbor",
    ] {
        let child = directory.join(name);
        match child.symlink_metadata() {
            Ok(metadata) => {
                if !metadata.is_file() || metadata.file_attributes() & 0x400 != 0 {
                    return Err("credential records must be ordinary files".into());
                }
                targets.push(child);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    // Replace existing record DACLs too; inherited directory rules alone would
    // leave old explicit grants on existing files intact.
    for target in targets {
        let path: Vec<u16> = target.as_os_str().encode_wide().chain([0]).collect();
        if path[..path.len() - 1].contains(&0) {
            return Err("invalid credential path".into());
        }
        // SAFETY: path is NUL-terminated; dacl remains valid until this loop ends.
        let status = unsafe {
            SetNamedSecurityInfoW(
                PCWSTR(path.as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                PSID::default(),
                PSID::default(),
                Some(dacl),
                None,
            )
        };
        if status != 0 {
            return Err(std::io::Error::from_raw_os_error(status as i32).into());
        }
    }
    Ok(())
}
