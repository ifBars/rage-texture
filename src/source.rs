//! Source snapshots are per-open-file, never keyed by an asset hash or filename alone.
use std::{
    fs::{File, Metadata},
    io,
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Stamp {
    len: u64,
    modified: SystemTime,
    created: Option<SystemTime>,
}
impl Stamp {
    fn of(m: Metadata) -> io::Result<Self> {
        Ok(Self {
            len: m.len(),
            modified: m.modified()?,
            created: m.created().ok(),
        })
    }
}
pub(crate) struct Source {
    path: PathBuf,
    identity: same_file::Handle,
    stamp: Stamp,
}
impl Source {
    pub(crate) fn capture(path: &Path, file: &File) -> io::Result<Self> {
        let path = std::path::absolute(path)?;
        let source = Self {
            path,
            identity: same_file::Handle::from_file(file.try_clone()?)?,
            stamp: Stamp::of(file.metadata()?)?,
        };
        source.validate(file)?;
        Ok(source)
    }
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
    pub(crate) fn validate(&self, file: &File) -> io::Result<()> {
        let identity = same_file::Handle::from_path(&self.path)?;
        if identity != self.identity
            || Stamp::of(file.metadata()?)? != self.stamp
            || Stamp::of(std::fs::metadata(&self.path)?)? != self.stamp
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "source changed or was replaced; refresh the store",
            ));
        }
        Ok(())
    }
}
