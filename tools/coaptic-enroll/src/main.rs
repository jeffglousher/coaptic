//! Cross-platform hardware-approved commissioning and device-key provisioning.
//! Optional Windows WebAuthn delegates user verification to Windows Security.
#[cfg(all(windows, feature = "webauthn"))]
use coaptic_enroll::{
    Request, begin_registration, origin,
    store::{self, OperatorStore},
};
use std::path::PathBuf;

#[cfg(feature = "development-piv")]
mod piv_cli;

mod pairing_cli;

mod owner_recovery_cli;
mod reconcile_cli;
mod serial_cli;

mod guided;
mod help;
mod inspect;

fn main() {
    if let Err(error) = run() {
        eprintln!("coaptic-enroll: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let supplied: Vec<_> = std::env::args_os().skip(1).collect();
    if supplied.is_empty() || supplied == [std::ffi::OsString::from("guide")] {
        println!(
            "Start with device inspection. New hosts need native storage preparation and an identity before pairing."
        );
        if let Ok(ports) = serialport::available_ports() {
            for port in ports {
                println!("Available serial port: {}", port.port_name);
            }
        }
        loop {
            let Some(args) = guided::collect(
                &mut std::io::stdin().lock(),
                &mut std::io::stdout().lock(),
                guided::Capabilities {
                    webauthn: cfg!(all(windows, feature = "webauthn")),
                    piv: cfg!(feature = "development-piv"),
                },
            )?
            else {
                return Ok(());
            };
            if let Err(error) = run_command(args) {
                eprintln!(
                    "Operation failed: {error}\nNo automatic retry. Inspect pairing state after an uncertain transport or storage outcome."
                );
            }
        }
    }
    run_command(supplied)
}

fn run_command(supplied: Vec<std::ffi::OsString>) -> Result<(), Box<dyn std::error::Error>> {
    if let [command, flag] = supplied.as_slice()
        && (flag == "--help" || flag == "-h")
        && let Some(usage) = help::command(command)
    {
        println!("{usage}");
        return Ok(());
    }
    let mut args = supplied.into_iter();
    let command = args
        .next()
        .ok_or("expected operator-init or operator-check REQUEST.cbor")?;
    if command == "--help" || command == "-h" {
        if args.next().is_some() {
            return Err("unexpected arguments after help".into());
        }
        println!(
            "Coaptic commissioning - run without arguments for the guided menu.
\
Inspect: inspect [--port PORT] [--host NAME]
\
  Local inventory is immediate; only the selected port is contacted (10-second limit).
\
Setup: custody-init | device-init NAME KID | serial-init PORT KID
\
Pair: serial-pair PORT HOST HOST_ADDRESS [--read-only | --echo-only] APPROVAL
  Qualification only: --qualify-lost-commissioning-reply discards its actual USB commit reply before saving the host pairing; see serial-pair --help.
\
Connect: peer HOST LOCAL_ADDRESS PEER_ADDRESS --connect
  Qualification only: --qualify-lost-receipt exits75 after first durable effect, before receipt.
  Qualification only: --qualify-queued-revoke PORT --development-no-touch ends revoked; see peer --help.
  Qualification only: --qualify-pressure runs three retained-TX refusal/recovery cycles without management or telemetry effects.
  Qualification only: --qualify-timing records four warmups and 24 protected identity timings in one session; no grant or journal changes.
  Qualification only: --qualify-device-restart withholds one receipt in this host process for a device-only restart (90s window).
\
Maintain: serial-quiesce PORT | serial-restart PORT | serial-recover PORT HOST APPROVAL
  serial-recover also accepts --approval-directory DIRECTORY for the current owner's credential.
  serial-reconcile PORT HOST PRIOR_PAIRING.cbor APPROVAL
  Receipt-only repair under an unchanged revoked policy; --help describes prerequisites.
\
  serial-revoke PORT HOST APPROVAL
\
  serial-transfer PORT NEW_HOST HOST_ADDRESS VERIFIED_OWNER_HEX [--read-only | --echo-only] APPROVAL
  serial-backup PORT BACKUP_DIRECTORY COSIGNER_MODE APPROVAL
  serial-backup-remove PORT APPROVAL
  serial-owner-recover PORT NEW_HOST HOST_ADDRESS NEW_OWNER_DIRECTORY COSIGNER_MODE [--read-only | --echo-only] APPROVAL
COSIGNER_MODE: webauthn | touch | touchless. Register credentials in separate directories
using COAPTIC_ENROLLMENT_DIR (an absolute path). Recovery requires a previously authorized
backup and proof by the destination owner; it never resets device identity.
\
Inspect individual records: serial-info PORT | device-info HOST | pairing-info HOST
\
Local pairing: pairing-init DEVICE | receipts-init DEVICE | pair DEVICE PEER APPROVAL | revoke DEVICE APPROVAL
\
  transfer DEVICE NEW_PEER VERIFIED_OWNER_HEX [--read-only | --echo-only] APPROVAL"
        );
        #[cfg(feature = "development-piv")]
        println!(
            "Credentials: piv-touch-register SERIAL SLOT PUBLIC.pem | piv-touch-test
\
  development-piv-register SERIAL SLOT PUBLIC.pem | development-piv-test
\
  piv-touch-check REQUEST.cbor | development-piv-check REQUEST.cbor
\
APPROVAL: --piv-touch or --development-no-touch (explicit development use)"
        );
        #[cfg(all(windows, feature = "webauthn"))]
        println!(
            "Windows WebAuthn: operator-init | operator-check REQUEST.cbor
APPROVAL: omit the PIV flag for Windows verified authentication."
        );
        return Ok(());
    }
    if command == "inspect" {
        return inspect::run(args, &enrollment_directory()?);
    }
    #[cfg(feature = "development-piv")]
    let development = [
        "development-piv-register",
        "development-piv-check",
        "piv-touch-register",
        "piv-touch-check",
        "piv-touch-test",
        "development-piv-test",
    ]
    .iter()
    .any(|name| command == *name);
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
        "receipts-init",
        "pairing-info",
        "peer",
        "serial-info",
        "serial-init",
        "serial-restart",
        "serial-quiesce",
        "serial-pair",
        "serial-revoke",
        "serial-transfer",
        "serial-recover",
        "serial-reconcile",
        "serial-backup",
        "serial-backup-remove",
        "serial-owner-recover",
        "custody-init",
    ]
    .iter()
    .any(|value| command == *value)
        && !development
    {
        return Err("expected operator-init or operator-check REQUEST.cbor".into());
    }
    let directory = enrollment_directory()?;
    private_directory(&directory)?;
    if command == "serial-reconcile" {
        return reconcile_cli::run(args, &directory);
    }
    if [
        "serial-backup",
        "serial-backup-remove",
        "serial-owner-recover",
    ]
    .iter()
    .any(|value| command == *value)
    {
        return owner_recovery_cli::run(&command, args, &directory);
    }
    if command == "custody-init" {
        if args.next().is_some() {
            return Err("unexpected arguments".into());
        }
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let _lock =
                coaptic_enroll::store::RecordStore::<u8>::open_named(&directory, "custody")?;
            coaptic_enroll::native_custody::initialize(&directory)?;
        }
        println!("Native credential custody ready.");
        return Ok(());
    }
    if [
        "serial-info",
        "serial-init",
        "serial-restart",
        "serial-quiesce",
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
        "receipts-init",
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
            coaptic_enroll::pairing::initialize_receipts(&device_directory, &identity)?;
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
    operator_command(command, args, &directory)
}

#[cfg(not(all(windows, feature = "webauthn")))]
fn operator_command(
    _: std::ffi::OsString,
    _: impl Iterator<Item = std::ffi::OsString>,
    _: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    Err("Windows WebAuthn is unavailable in this build; select an explicitly registered PIV approval profile".into())
}

#[cfg(all(windows, feature = "webauthn"))]
fn operator_command(
    command: std::ffi::OsString,
    mut args: impl Iterator<Item = std::ffi::OsString>,
    directory: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    use webauthn_authenticator_rs::{WebauthnAuthenticator, win10::Win10};
    // Ordinary device-key recovery does not wait on an operator ceremony lock.
    let operator_store = OperatorStore::open(directory)?;
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

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn enrollment_directory() -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Some(path) = std::env::var_os("COAPTIC_ENROLLMENT_DIR") {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err("COAPTIC_ENROLLMENT_DIR must be absolute".into());
        }
        return Ok(path);
    }
    #[cfg(windows)]
    {
        Ok(
            PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA unavailable")?)
                .join("Coaptic")
                .join("Enrollment"),
        )
    }
    #[cfg(target_os = "macos")]
    {
        Ok(
            PathBuf::from(std::env::var_os("HOME").ok_or("HOME unavailable")?)
                .join("Library/Application Support/Coaptic/Enrollment"),
        )
    }
    #[cfg(target_os = "linux")]
    {
        let root = match std::env::var_os("XDG_DATA_HOME") {
            Some(path) if PathBuf::from(&path).is_absolute() => PathBuf::from(path),
            _ => PathBuf::from(std::env::var_os("HOME").ok_or("HOME unavailable")?)
                .join(".local/share"),
        };
        Ok(root.join("coaptic/enrollment"))
    }
}

#[cfg(unix)]
fn private_directory(directory: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    // Refuse existing symlinks before recursive creation can follow an ancestor.
    for ancestor in directory.ancestors() {
        match ancestor.symlink_metadata() {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("operator directory ancestors must not be symlinks".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder.create(directory)?;
    let metadata = directory.symlink_metadata()?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() || metadata.mode() & 0o077 != 0 {
        return Err("enrollment directory must be a private directory (mode 0700)".into());
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod private_directory_tests {
    use super::private_directory;
    use std::os::unix::fs::{MetadataExt, symlink};

    struct TestDirectory(std::path::PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let mut nonce = [0; 8];
            getrandom::fill(&mut nonce).unwrap();
            // macOS's temporary directory can itself have a symlink ancestor.
            let directory = std::env::temp_dir().canonicalize().unwrap().join(format!(
                "coaptic-private-directory-{}-{:016x}",
                std::process::id(),
                u64::from_ne_bytes(nonce)
            ));
            std::fs::create_dir(&directory).unwrap();
            Self(directory)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn symlink_ancestor_is_refused_before_creating_children() {
        let root = TestDirectory::new();
        let target = root.0.join("target");
        std::fs::create_dir(&target).unwrap();
        let link = root.0.join("link");
        symlink(&target, &link).unwrap();

        assert!(private_directory(&link.join("new/enrollment")).is_err());
        assert!(!target.join("new").exists());
        assert!(private_directory(&link).is_err());
    }

    #[test]
    fn ordinary_private_directory_can_be_created_and_reopened() {
        let root = TestDirectory::new();
        let directory = root.0.join("new/enrollment");
        private_directory(&directory).unwrap();
        private_directory(&directory).unwrap();
        assert_eq!(directory.symlink_metadata().unwrap().mode() & 0o777, 0o700);
    }
}

fn device_directory(
    directory: &std::path::Path,
    name: &std::ffi::OsStr,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let result = existing_device_directory(directory, name)?;
    private_directory(&result)?;
    Ok(result)
}

fn existing_device_directory(
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
    if !result.symlink_metadata()?.file_type().is_dir() {
        return Err("device is not provisioned".into());
    }
    Ok(result)
}

#[cfg(windows)]
fn process_user_sid() -> Result<String, Box<dyn std::error::Error>> {
    use windows::Win32::{
        Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER, HANDLE},
        Security::{
            Authorization::ConvertSidToStringSidW, GetTokenInformation, TOKEN_QUERY, TOKEN_USER,
            TokenUser,
        },
        System::{
            Memory::LocalFree,
            Threading::{GetCurrentProcess, OpenProcessToken},
        },
    };
    use windows::core::PWSTR;

    struct Token(HANDLE);
    impl Drop for Token {
        fn drop(&mut self) {
            // SAFETY: this wrapper owns the handle returned by OpenProcessToken.
            unsafe { CloseHandle(self.0) };
        }
    }
    struct SidString(PWSTR);
    impl Drop for SidString {
        fn drop(&mut self) {
            // SAFETY: ConvertSidToStringSidW allocates with LocalAlloc.
            unsafe { LocalFree(self.0.0 as isize) };
        }
    }

    let mut handle = HANDLE::default();
    // SAFETY: the process pseudo-handle is valid and handle is writable.
    if !unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut handle) }.as_bool() {
        return Err(std::io::Error::last_os_error().into());
    }
    let token = Token(handle);
    let mut needed = 0;
    // SAFETY: the first call queries the required buffer length only.
    let queried = unsafe { GetTokenInformation(token.0, TokenUser, None, 0, &mut needed) };
    let error = std::io::Error::last_os_error();
    if !queried.as_bool() && error.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER.0 as i32) {
        return Err(error.into());
    }
    if queried.as_bool() || needed < std::mem::size_of::<TOKEN_USER>() as u32 {
        return Err("invalid OS token user buffer length".into());
    }
    // TOKEN_USER contains a pointer, so byte-vector alignment is insufficient.
    let mut buffer = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
    // SAFETY: buffer is aligned for TOKEN_USER and contains at least needed bytes.
    if !unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            Some(buffer.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
    }
    .as_bool()
    {
        return Err(std::io::Error::last_os_error().into());
    }
    // SAFETY: the successful query initialized TOKEN_USER and its SID in buffer.
    let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
    let mut sid = PWSTR::null();
    // SAFETY: the returned SID remains valid while buffer is alive.
    if !unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid) }.as_bool() {
        return Err(std::io::Error::last_os_error().into());
    }
    let sid = SidString(sid);
    // SAFETY: ConvertSidToStringSidW returned a NUL-terminated UTF-16 string.
    Ok(unsafe { sid.0.to_string() }?)
}

#[cfg(all(test, windows))]
#[test]
fn process_user_sid_reads_native_token() {
    let sid = process_user_sid().expect("current process user SID");
    assert!(sid.starts_with("S-1-"));
    assert!(
        sid[2..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'-')
    );
}

#[cfg(windows)]
fn private_directory(directory: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
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
    let sid = process_user_sid()?;
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
        "piv-touch.lock",
        "piv-touch.cbor",
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
