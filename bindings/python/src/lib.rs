//! Optional PyO3 bindings for the `rage-texture` core.
//!
//! Experimental and separately built. This crate does not change the core
//! library; it wraps its public read-only session and extraction APIs.
//! GPL-3.0-only, matching the combined project.

use pyo3::create_exception;
use pyo3::exceptions::{
    PyException, PyFileExistsError, PyFileNotFoundError, PyOSError, PyPermissionError,
};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyModule};
use rage_texture::catalog::Texture;
use rage_texture::extract::ExtractionTimings;
use rage_texture::{DecodePolicy, Quality, Stats, Store};
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, TryLockError};
use std::time::Instant;

create_exception!(_native, Error, PyException);
create_exception!(_native, UnsupportedError, Error);
create_exception!(_native, CorruptError, Error);
create_exception!(_native, StaleError, Error);
create_exception!(_native, ClosedError, Error);
create_exception!(_native, BusyError, Error);
create_exception!(_native, BoundsError, Error);

fn closed_error() -> PyErr {
    ClosedError::new_err("store is closed")
}

fn busy_error() -> PyErr {
    BusyError::new_err("store is busy in another thread; retry after the current operation")
}

fn lock_error<T>(error: TryLockError<T>) -> PyErr {
    match error {
        TryLockError::WouldBlock => busy_error(),
        TryLockError::Poisoned(_) => Error::new_err("store lock is poisoned"),
    }
}

/// Map core `io::Error` values onto explicit Python exceptions. Defaults are
/// deliberately not used for unsupported, corrupt, stale or out-of-range
/// conditions.
fn map_io(error: io::Error) -> PyErr {
    let message = error.to_string();
    match error.kind() {
        io::ErrorKind::Unsupported => UnsupportedError::new_err(message),
        io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof => {
            if message.contains("stale")
                || message.contains("another source generation")
                || message.contains("source changed")
                || message.contains("refresh the store")
            {
                StaleError::new_err(message)
            } else if message.starts_with("range ")
                || message.contains("out of bounds")
                || message.contains("outside the layout")
            {
                BoundsError::new_err(message)
            } else {
                CorruptError::new_err(message)
            }
        }
        io::ErrorKind::InvalidInput => BoundsError::new_err(message),
        io::ErrorKind::NotFound => PyFileNotFoundError::new_err(message),
        io::ErrorKind::PermissionDenied => PyPermissionError::new_err(message),
        io::ErrorKind::AlreadyExists => PyFileExistsError::new_err(message),
        _ => PyOSError::new_err(message),
    }
}

enum HistoryInput {
    Path(PathBuf),
    Json(Vec<u8>),
}
impl HistoryInput {
    fn from_python(py: Python<'_>, value: &Bound<'_, PyAny>) -> PyResult<Self> {
        if value.is_instance_of::<PyDict>() {
            let json = py
                .import("json")?
                .call_method1("dumps", (value,))?
                .extract::<String>()?;
            Ok(Self::Json(json.into_bytes()))
        } else {
            Ok(Self::Path(value.extract::<PathBuf>()?))
        }
    }
    fn load(self) -> io::Result<Arc<rage_texture::HistoryTables>> {
        let tables = match self {
            Self::Path(path) => rage_texture::HistoryTables::from_path(path)?,
            Self::Json(bytes) => rage_texture::HistoryTables::from_json(&bytes)?,
        };
        Ok(Arc::new(tables))
    }
}

type SharedStore = Arc<Mutex<Option<Store>>>;

/// Run one store operation without the GIL. The mutex is acquired inside the
/// detached closure and never held while Python code runs. A second operation
/// on the same store therefore fails with `BusyError` instead of blocking or
/// creating overlapping `&mut Store` borrows.
fn run<T, F>(py: Python<'_>, shared: &SharedStore, operation: F) -> PyResult<T>
where
    T: Send + 'static,
    F: Send + 'static + FnOnce(&mut Store) -> io::Result<T>,
{
    let shared = Arc::clone(shared);
    py.detach(move || {
        let mut guard = shared.try_lock().map_err(lock_error)?;
        let store = guard.as_mut().ok_or_else(closed_error)?;
        operation(store).map_err(map_io)
    })
}

#[pyclass(frozen, module = "rage_texture._native", name = "Quality", get_all)]
#[derive(Clone)]
struct PyQuality {
    kind: String,
    codec: Option<u8>,
    unfitted_blocks: usize,
    invalid_blocks: Option<usize>,
    history_digest: Option<String>,
    provisional_dependencies: usize,
}
impl From<Quality> for PyQuality {
    fn from(value: Quality) -> Self {
        let mut result = Self {
            kind: "lossless".into(),
            codec: None,
            unfitted_blocks: 0,
            invalid_blocks: Some(0),
            history_digest: None,
            provisional_dependencies: 0,
        };
        match value {
            Quality::LosslessContainer => {}
            Quality::FittedBc1 { unfitted_blocks } => {
                result.kind = "fitted_bc1".into();
                result.codec = Some(1);
                result.unfitted_blocks = unfitted_blocks;
            }
            Quality::FittedBc {
                codec,
                unfitted_blocks,
                invalid_blocks,
            } => {
                result.kind = "fitted_bc".into();
                result.codec = Some(codec);
                result.unfitted_blocks = unfitted_blocks;
                result.invalid_blocks = Some(invalid_blocks);
            }
            Quality::HistoryBc {
                codec,
                provisional_dependencies,
                history_digest,
            } => {
                result.kind = if provisional_dependencies == 0 {
                    "confirmed_history"
                } else {
                    "provisional_history"
                }
                .into();
                result.codec = Some(codec);
                result.provisional_dependencies = provisional_dependencies;
                result.invalid_blocks = if provisional_dependencies == 0 {
                    Some(0)
                } else {
                    None
                };
                result.history_digest =
                    Some(history_digest.iter().map(|b| format!("{b:02x}")).collect());
            }
        }
        result
    }
}

#[pymethods]
impl PyQuality {
    #[getter]
    fn is_lossless(&self) -> bool {
        self.kind == "lossless"
    }
    #[getter]
    fn is_confirmed(&self) -> bool {
        matches!(self.kind.as_str(), "lossless" | "confirmed_history")
    }

    fn __repr__(&self) -> String {
        format!(
            "Quality(kind={:?}, codec={:?}, unfitted_blocks={}, invalid_blocks={:?})",
            self.kind, self.codec, self.unfitted_blocks, self.invalid_blocks
        )
    }
}

#[pyclass(frozen, module = "rage_texture._native", name = "Stats", get_all)]
#[derive(Clone, Copy)]
struct PyStats {
    cache_hits: u64,
    cache_misses: u64,
    decodes: u64,
    source_bytes: u64,
    cache_bytes: usize,
    peak_cache_bytes: usize,
    evictions: u64,
}

impl From<Stats> for PyStats {
    fn from(value: Stats) -> Self {
        Self {
            cache_hits: value.cache_hits,
            cache_misses: value.cache_misses,
            decodes: value.decodes,
            source_bytes: value.source_bytes,
            cache_bytes: value.cache_bytes,
            peak_cache_bytes: value.peak_cache_bytes,
            evictions: value.evictions,
        }
    }
}

#[pymethods]
impl PyStats {
    fn __repr__(&self) -> String {
        format!(
            "Stats(hits={}, misses={}, decodes={}, source_bytes={}, cache_bytes={})",
            self.cache_hits, self.cache_misses, self.decodes, self.source_bytes, self.cache_bytes
        )
    }
}

#[pyclass(frozen, module = "rage_texture._native", name = "Export")]
struct PyExport {
    data: Py<PyBytes>,
    data_len: usize,
    quality: Vec<PyQuality>,
    input_bytes: u64,
    read_decode_seconds: f64,
    detile_seconds: f64,
    dds_seconds: f64,
    encode_seconds: f64,
}

impl PyExport {
    fn from_parts(
        py: Python<'_>,
        data: Vec<u8>,
        quality: Vec<Quality>,
        input_bytes: u64,
        timings: ExtractionTimings,
        encode_seconds: f64,
    ) -> Self {
        Self {
            data: PyBytes::new(py, &data).unbind(),
            data_len: data.len(),
            quality: quality.into_iter().map(PyQuality::from).collect(),
            input_bytes,
            read_decode_seconds: timings.read_decode_seconds,
            detile_seconds: timings.detile_seconds,
            dds_seconds: timings.dds_seconds,
            encode_seconds,
        }
    }
}

#[pymethods]
impl PyExport {
    #[getter]
    fn data(&self, py: Python<'_>) -> Py<PyBytes> {
        self.data.clone_ref(py)
    }

    #[getter]
    fn quality(&self) -> Vec<PyQuality> {
        self.quality.clone()
    }

    #[getter]
    fn input_bytes(&self) -> u64 {
        self.input_bytes
    }

    #[getter]
    fn read_decode_seconds(&self) -> f64 {
        self.read_decode_seconds
    }

    #[getter]
    fn detile_seconds(&self) -> f64 {
        self.detile_seconds
    }

    #[getter]
    fn dds_seconds(&self) -> f64 {
        self.dds_seconds
    }

    #[getter]
    fn encode_seconds(&self) -> f64 {
        self.encode_seconds
    }

    #[getter]
    fn total_seconds(&self) -> f64 {
        self.read_decode_seconds + self.detile_seconds + self.dds_seconds + self.encode_seconds
    }

    fn __repr__(&self) -> String {
        format!(
            "Export(bytes={}, quality={}, input_bytes={})",
            self.data_len,
            self.quality.len(),
            self.input_bytes
        )
    }
}

#[pyclass(frozen, module = "rage_texture._native", name = "Texture")]
#[derive(Clone)]
struct PyTexture {
    inner: Texture,
}

impl From<Texture> for PyTexture {
    fn from(inner: Texture) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl PyTexture {
    #[getter]
    fn index(&self) -> usize {
        self.inner.index
    }

    #[getter]
    fn asset_key(&self) -> Option<u64> {
        self.inner.asset_key
    }

    #[getter]
    fn width(&self) -> u32 {
        self.inner.width
    }

    #[getter]
    fn height(&self) -> u32 {
        self.inner.height
    }

    #[getter]
    fn depth(&self) -> u32 {
        self.inner.depth
    }

    #[getter]
    fn mips(&self) -> u32 {
        self.inner.mips
    }

    #[getter]
    fn dxgi(&self) -> u32 {
        self.inner.dxgi
    }

    #[getter]
    fn dimension(&self) -> u32 {
        self.inner.dimension
    }

    #[getter]
    fn mode(&self) -> u32 {
        self.inner.mode
    }

    #[getter]
    fn non_tail(&self) -> u32 {
        self.inner.non_tail
    }

    #[getter]
    fn base(&self) -> u64 {
        self.inner.base
    }

    #[getter]
    fn meta(&self) -> u32 {
        self.inner.meta
    }

    #[getter]
    fn extra(&self) -> u32 {
        self.inner.extra
    }

    fn __repr__(&self) -> String {
        format!(
            "Texture(index={}, {}x{}x{}, dxgi={}, mips={}, dimension={}, mode={})",
            self.inner.index,
            self.inner.width,
            self.inner.height,
            self.inner.depth,
            self.inner.dxgi,
            self.inner.mips,
            self.inner.dimension,
            self.inner.mode
        )
    }
}

#[pyclass(module = "rage_texture._native", name = "Store")]
struct PyStore {
    path: PathBuf,
    inner: SharedStore,
}

#[pymethods]
impl PyStore {
    #[new]
    #[pyo3(signature = (path, cache_bytes=0, allow_fitted=false, *, decode_policy=None, history=None))]
    fn new(
        py: Python<'_>,
        path: PathBuf,
        cache_bytes: i64,
        allow_fitted: bool,
        decode_policy: Option<&str>,
        history: Option<&Bound<'_, PyAny>>,
    ) -> PyResult<Self> {
        if cache_bytes < 0 {
            return Err(BoundsError::new_err("cache_bytes must be >= 0"));
        }
        let budget = usize::try_from(cache_bytes)
            .map_err(|_| BoundsError::new_err("cache_bytes is too large for this platform"))?;
        let policy = match decode_policy {
            Some("lossless") => DecodePolicy::LosslessOnly,
            Some("confirmed") => DecodePolicy::ConfirmedOnly,
            Some("provisional") => DecodePolicy::AllowProvisional,
            Some("fitted") => DecodePolicy::AllowFittedBc,
            Some(_) => {
                return Err(BoundsError::new_err(
                    "decode_policy must be lossless, confirmed, provisional or fitted",
                ));
            }
            None if history.is_some() => DecodePolicy::ConfirmedOnly,
            None if allow_fitted => DecodePolicy::AllowFittedBc,
            None => DecodePolicy::LosslessOnly,
        };
        if allow_fitted && policy != DecodePolicy::AllowFittedBc {
            return Err(BoundsError::new_err(
                "allow_fitted conflicts with history decode policy",
            ));
        }
        if history.is_some()
            && !matches!(
                policy,
                DecodePolicy::ConfirmedOnly | DecodePolicy::AllowProvisional
            )
        {
            return Err(BoundsError::new_err(
                "history requires confirmed or provisional decode policy",
            ));
        }
        let history = history
            .map(|value| HistoryInput::from_python(py, value))
            .transpose()?;
        let open_path = path.clone();
        let store = py
            .detach(move || match history {
                Some(input) => Store::open_with_history(open_path, budget, policy, input.load()?),
                None => Store::open(open_path, budget, policy),
            })
            .map_err(map_io)?;
        Ok(Self {
            path,
            inner: Arc::new(Mutex::new(Some(store))),
        })
    }

    #[getter]
    fn closed(&self) -> bool {
        self.inner
            .try_lock()
            .map(|guard| guard.is_none())
            .unwrap_or(false)
    }

    fn textures(&self, py: Python<'_>) -> PyResult<Vec<PyTexture>> {
        run(py, &self.inner, |store| {
            store
                .textures()
                .map(|values| values.into_iter().map(PyTexture::from).collect())
        })
    }

    #[pyo3(signature = (texture, mips=None))]
    fn texture_dds(
        &self,
        py: Python<'_>,
        texture: PyRef<'_, PyTexture>,
        mips: Option<u32>,
    ) -> PyResult<PyExport> {
        let texture = texture.inner.clone();
        let (data, quality, input_bytes, timings) = run(py, &self.inner, move |store| {
            let extracted = store.texture_dds_mips(&texture, mips.unwrap_or(texture.mips))?;
            Ok((
                extracted.dds,
                extracted.quality,
                extracted.input_bytes,
                extracted.timings,
            ))
        })?;
        Ok(PyExport::from_parts(
            py,
            data,
            quality,
            input_bytes,
            timings,
            0.0,
        ))
    }

    #[pyo3(signature = (texture, mip=0, slice=0))]
    fn texture_mip(
        &self,
        py: Python<'_>,
        texture: PyRef<'_, PyTexture>,
        mip: u32,
        slice: u32,
    ) -> PyResult<PyExport> {
        let texture = texture.inner.clone();
        let (data, quality, input_bytes, elapsed) = run(py, &self.inner, move |store| {
            let layout = store.texture_layout(&texture)?;
            let input_bytes = layout.mip(mip)?.region;
            let started = Instant::now();
            let (data, quality) = store.texture_mip(&texture, mip, slice)?;
            let elapsed = started.elapsed().as_secs_f64();
            Ok((data, quality, input_bytes, elapsed))
        })?;
        Ok(PyExport::from_parts(
            py,
            data,
            quality,
            input_bytes,
            ExtractionTimings {
                read_decode_seconds: elapsed,
                ..ExtractionTimings::default()
            },
            0.0,
        ))
    }

    #[pyo3(signature = (texture, mip=0, slice=0))]
    fn texture_png(
        &self,
        py: Python<'_>,
        texture: PyRef<'_, PyTexture>,
        mip: u32,
        slice: u32,
    ) -> PyResult<PyExport> {
        let texture = texture.inner.clone();
        let (data, quality, input_bytes, elapsed) = run(py, &self.inner, move |store| {
            let layout = store.texture_layout(&texture)?;
            let input_bytes = layout.mip(mip)?.region;
            let started = Instant::now();
            let (data, quality) = store.texture_png(&texture, mip, slice)?;
            let elapsed = started.elapsed().as_secs_f64();
            Ok((data, quality, input_bytes, elapsed))
        })?;
        Ok(PyExport::from_parts(
            py,
            data,
            quality,
            input_bytes,
            ExtractionTimings::default(),
            elapsed,
        ))
    }

    fn read_range(&self, py: Python<'_>, start: u64, length: usize) -> PyResult<Py<PyBytes>> {
        let bytes = run(py, &self.inner, move |store| {
            store.read_range(start, length).map(|(bytes, _)| bytes)
        })?;
        Ok(PyBytes::new(py, &bytes).unbind())
    }

    fn read_range_result(&self, py: Python<'_>, start: u64, length: usize) -> PyResult<PyExport> {
        let (data, quality, elapsed) = run(py, &self.inner, move |store| {
            let started = Instant::now();
            let (bytes, quality) = store.read_range(start, length)?;
            Ok((bytes, quality, started.elapsed().as_secs_f64()))
        })?;
        Ok(PyExport::from_parts(
            py,
            data,
            quality,
            length as u64,
            ExtractionTimings {
                read_decode_seconds: elapsed,
                ..ExtractionTimings::default()
            },
            0.0,
        ))
    }

    fn stats(&self, py: Python<'_>) -> PyResult<PyStats> {
        run(py, &self.inner, |store| Ok(store.stats())).map(PyStats::from)
    }

    fn reset(&self, py: Python<'_>) -> PyResult<()> {
        run(py, &self.inner, |store| {
            store.reset();
            Ok(())
        })
    }

    fn refresh(&self, py: Python<'_>) -> PyResult<()> {
        run(py, &self.inner, |store| store.refresh())
    }
    fn replace_history(&self, py: Python<'_>, history: &Bound<'_, PyAny>) -> PyResult<()> {
        let input = HistoryInput::from_python(py, history)?;
        run(py, &self.inner, move |store| {
            store.replace_history(input.load()?)
        })
    }
    #[getter]
    fn history_digest(&self, py: Python<'_>) -> PyResult<Option<String>> {
        run(py, &self.inner, |store| {
            Ok(store
                .history_digest()
                .map(|d| d.iter().map(|b| format!("{b:02x}")).collect()))
        })
    }

    fn close(&self, py: Python<'_>) -> PyResult<()> {
        let shared = Arc::clone(&self.inner);
        py.detach(move || {
            let mut guard = shared.try_lock().map_err(lock_error)?;
            guard.take();
            Ok(())
        })
    }

    fn __enter__(slf: PyRef<'_, Self>) -> PyResult<PyRef<'_, Self>> {
        if slf.closed() {
            return Err(closed_error());
        }
        Ok(slf)
    }

    fn __exit__(
        &self,
        py: Python<'_>,
        _exc_type: &Bound<'_, PyAny>,
        _exc_value: &Bound<'_, PyAny>,
        _traceback: &Bound<'_, PyAny>,
    ) -> PyResult<bool> {
        self.close(py)?;
        Ok(false)
    }

    fn __repr__(&self) -> String {
        format!(
            "Store(path={:?}, closed={})",
            self.path.display(),
            self.closed()
        )
    }
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add_class::<PyStore>()?;
    module.add_class::<PyTexture>()?;
    module.add_class::<PyExport>()?;
    module.add_class::<PyQuality>()?;
    module.add_class::<PyStats>()?;

    let py = module.py();
    module.add("Error", py.get_type::<Error>())?;
    module.add("UnsupportedError", py.get_type::<UnsupportedError>())?;
    module.add("CorruptError", py.get_type::<CorruptError>())?;
    module.add("StaleError", py.get_type::<StaleError>())?;
    module.add("ClosedError", py.get_type::<ClosedError>())?;
    module.add("BusyError", py.get_type::<BusyError>())?;
    module.add("BoundsError", py.get_type::<BoundsError>())?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
