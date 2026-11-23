//! Complete-file publication without overwriting existing data files.
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
pub fn publish_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    publish(path, bytes, false)
}
/// Reuse only byte-identical output, never a timestamp or filename heuristic.
pub fn publish_or_verify(path: &Path, bytes: &[u8]) -> io::Result<()> {
    match fs::read(path) {
        Ok(existing) if existing == bytes => Ok(()),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "existing output differs from freshly decoded source; use a new output directory",
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => publish_new(path, bytes),
        Err(e) => Err(e),
    }
}
/// Replace only a manifest carrying our generator marker. Data files remain protected.
pub fn publish_manifest(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if path.exists() {
        let previous: serde_json::Value = serde_json::from_slice(&fs::read(path)?)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        if previous["generator"] != "rage-texture" {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "existing manifest is not owned by rage-texture",
            ));
        }
    }
    publish(path, bytes, true)
}
fn publish(path: &Path, bytes: &[u8], replace: bool) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    if replace {
        temp.persist(path).map_err(io::Error::from)?;
    } else {
        temp.persist_noclobber(path).map_err(io::Error::from)?;
    }
    Ok(())
}
