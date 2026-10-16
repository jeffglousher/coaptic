use coaptic_enroll::{
    Request,
    development_piv::{self, DevelopmentOperator},
    store::{self, RecordStore},
};
use openssl::{bn::BigNumContext, ec::PointConversionForm, nid::Nid, pkey::PKey};
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

pub fn run(
    command: &OsStr,
    mut args: impl Iterator<Item = std::ffi::OsString>,
    directory: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let store = RecordStore::<DevelopmentOperator>::open_named(directory, "piv-development")?;
    if command == "development-piv-register" {
        if store.exists() {
            return Err("development operator already registered".into());
        }
        let serial: u32 = args
            .next()
            .ok_or("missing YubiKey serial")?
            .to_str()
            .ok_or("invalid serial")?
            .parse()?;
        let slot = args.next().ok_or("missing retired PIV slot")?;
        let slot = u8::from_str_radix(slot.to_str().ok_or("invalid slot")?, 16)?;
        let path = PathBuf::from(args.next().ok_or("missing public key PEM")?);
        if args.next().is_some() || std::fs::metadata(&path)?.len() > 16384 {
            return Err("unexpected arguments or oversized public key".into());
        }
        let key = PKey::public_key_from_pem(&std::fs::read(path)?)?.ec_key()?;
        if key.group().curve_name() != Some(Nid::X9_62_PRIME256V1) {
            return Err("development PIV key must be P-256".into());
        }
        let mut context = BigNumContext::new()?;
        let public = key.public_key().to_bytes(
            key.group(),
            PointConversionForm::UNCOMPRESSED,
            &mut context,
        )?;
        let pending = DevelopmentOperator::registration(serial, slot, &public)?;
        let proof = development_piv::sign(serial, slot, pending.digest())?;
        let operator = pending.finish(&proof)?;
        store.initialize(&operator)?;
        println!(
            "Development PIV operator registered: {}",
            super::hex(&operator.owner()?)
        );
        println!("No-touch development profile; production FIDO2 approval is separate.");
    } else {
        let path = PathBuf::from(args.next().ok_or("missing REQUEST.cbor")?);
        if args.next().is_some() {
            return Err("unexpected arguments".into());
        }
        let request: Request = store::read(&path)?;
        let commitment = request.commitment()?;
        let operator = store.load()?;
        let pending = operator.begin(request)?;
        let proof = development_piv::sign(operator.serial(), operator.slot(), pending.digest())?;
        let approval =
            pending.finish(&proof, |old, next| store.commit(old, next).map_err(|_| ()))?;
        approval.commit(|request| {
            if request.commitment()? != commitment {
                return Err(coaptic_enroll::Error::Input);
            }
            Ok(())
        })?;
        println!(
            "Development hardware proof verified and revision persisted: {}",
            super::hex(&commitment)
        );
    }
    Ok(())
}
