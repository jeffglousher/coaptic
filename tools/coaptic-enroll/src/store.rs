//! Exclusive bounded public operator storage. It is not a device trust anchor.
//! Sync/readback protect process recovery, not hostile whole-disk rollback.
use crate::Error;
#[cfg(feature = "webauthn")]
use crate::Operator;
use serde::{Serialize, de::DeserializeOwned};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const MAX_RECORD: u64 = 16 * 1024;

pub fn encode(value: &impl Serialize) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    ciborium::into_writer(value, &mut bytes).map_err(|_| Error::Persistence)?;
    if bytes.len() as u64 > MAX_RECORD {
        return Err(Error::Input);
    }
    Ok(bytes)
}

pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Error> {
    if bytes.len() as u64 > MAX_RECORD {
        return Err(Error::Input);
    }
    let mut input = bytes;
    let value =
        ciborium::de::from_reader_with_recursion_limit(&mut input, 32).map_err(|_| Error::Input)?;
    if !input.is_empty() {
        return Err(Error::Input);
    }
    Ok(value)
}

pub fn read<T: DeserializeOwned>(path: &Path) -> Result<T, Error> {
    let file = File::open(path).map_err(|_| Error::Persistence)?;
    if file.metadata().map_err(|_| Error::Persistence)?.len() > MAX_RECORD {
        return Err(Error::Input);
    }
    let mut bytes = Vec::new();
    file.take(MAX_RECORD + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Persistence)?;
    decode(&bytes)
}

#[cfg(feature = "webauthn")]
pub type OperatorStore = RecordStore<Operator>;

pub struct RecordStore<T> {
    _lock: File,
    path: PathBuf,
    _record: std::marker::PhantomData<T>,
}

#[cfg(feature = "webauthn")]
impl OperatorStore {
    /// `directory` must already be private to the authenticated OS administrator.
    pub fn open(directory: &Path) -> Result<Self, Error> {
        Self::open_named(directory, "operator")
    }
}

impl<T: Serialize + DeserializeOwned> RecordStore<T> {
    /// Open one bounded record under its exclusive process lock.
    pub fn open_named(directory: &Path, stem: &str) -> Result<Self, Error> {
        if stem.is_empty()
            || stem.len() > 32
            || !stem
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(Error::Input);
        }
        if !directory.is_dir()
            || directory
                .symlink_metadata()
                .map_err(|_| Error::Persistence)?
                .file_type()
                .is_symlink()
        {
            return Err(Error::Persistence);
        }
        let lock_name = format!("{stem}.lock");
        let record_name = format!("{stem}.cbor");
        for name in [&lock_name, &record_name] {
            match directory.join(name).symlink_metadata() {
                Ok(metadata) => {
                    #[cfg(windows)]
                    {
                        use std::os::windows::fs::MetadataExt;
                        if metadata.file_attributes() & 0x400 != 0 {
                            return Err(Error::Persistence);
                        }
                    }
                    if !metadata.is_file() {
                        return Err(Error::Persistence);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(Error::Persistence),
            }
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(lock_name))
            .map_err(|_| Error::Persistence)?;
        lock.try_lock().map_err(|_| Error::Persistence)?;
        Ok(Self {
            _lock: lock,
            path: directory.join(record_name),
            _record: std::marker::PhantomData,
        })
    }
    pub fn exists(&self) -> bool {
        self.path.exists()
    }
    pub fn load(&self) -> Result<T, Error> {
        read(&self.path)
    }

    pub fn initialize(&self, operator: &T) -> Result<(), Error> {
        let bytes = encode(operator)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&self.path).map_err(|_| Error::Persistence)?;
        write_and_verify(file, &bytes)
    }

    /// Compare exact prior state; confirm the updated counter before approval.
    pub fn commit(&self, old: &T, new: &T) -> Result<(), Error> {
        let old_bytes = encode(old)?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)
            .map_err(|_| Error::Persistence)?;
        let mut current = Vec::new();
        (&mut file)
            .take(MAX_RECORD + 1)
            .read_to_end(&mut current)
            .map_err(|_| Error::Persistence)?;
        if current != old_bytes {
            return Err(Error::Persistence);
        }
        write_and_verify(file, &encode(new)?)
    }
}

pub(crate) fn write_and_verify(mut file: File, bytes: &[u8]) -> Result<(), Error> {
    file.seek(SeekFrom::Start(0))
        .map_err(|_| Error::Persistence)?;
    file.write_all(bytes).map_err(|_| Error::Persistence)?;
    file.set_len(bytes.len() as u64)
        .map_err(|_| Error::Persistence)?;
    file.sync_all().map_err(|_| Error::Persistence)?;
    file.seek(SeekFrom::Start(0))
        .map_err(|_| Error::Persistence)?;
    let mut observed = Vec::new();
    file.take(MAX_RECORD + 1)
        .read_to_end(&mut observed)
        .map_err(|_| Error::Persistence)?;
    if observed != bytes {
        return Err(Error::Persistence);
    }
    Ok(())
}
