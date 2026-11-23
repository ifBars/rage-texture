"""Experimental, history-aware DDS backend for Leonida's tTPF tools.

Use explicitly at the export call site; no monkey-patching or fallback hides a
failure. The existing Python reader remains responsible for its own analysis.
The history-aware decoder preserves strict policy and supports custom tables
and leading partial mip chains. Install bindings/python first. GPL-3.0-only.
"""
from pathlib import Path

from rage_texture import Store, UnsupportedError


class NativeDdsBackend:
    """One source generation per backend. Create a new backend after updates.

    Use one backend per worker thread. The Store's decoded cache is disabled;
    its parsed catalog is retained. All source checks still run per extraction.
    """

    def __init__(self, path, *, history=None, allow_provisional=False):
        self.path = Path(path).resolve()
        self.allow_provisional = bool(allow_provisional)
        self.store = Store(self.path, cache_bytes=0,
                           decode_policy='provisional' if self.allow_provisional else 'confirmed',
                           history=history)
        try:
            self.catalog = self.store.textures()
        except BaseException:
            self.store.close()
            raise

    def export(self, pool, analysis, mips=None, *, allow_provisional=None, history=None):
        if Path(pool.path).resolve() != self.path:
            raise ValueError('analysis belongs to a different pool')
        if allow_provisional is not None and bool(allow_provisional) != self.allow_provisional:
            raise UnsupportedError('construct a backend with the requested provisional policy')
        if history is not None:
            # A per-call table override is loaded afresh, not keyed by path or
            # timestamp. Constructor history avoids this repeated setup cost.
            previous = self.store.history_digest
            self.store.replace_history(history)
            if self.store.history_digest != previous:
                self.catalog = self.store.textures()
        if 'blocked' in analysis or 'layout' not in analysis or 'available' not in analysis:
            raise UnsupportedError(analysis.get('blocked', 'analysis is not exportable'))
        descriptor = analysis['descriptor']
        index = descriptor['index']
        if not isinstance(index, int) or not 0 <= index < len(self.catalog):
            raise ValueError('descriptor index outside native catalog')
        texture = self.catalog[index]
        count = texture.mips if mips is None else mips
        if not isinstance(count, int) or not 1 <= count <= texture.mips:
            raise ValueError('DDS mip count is outside the source chain')
        fields = ('width', 'height', 'depth', 'mips', 'dimension', 'mode',
                  'non_tail', 'base', 'meta', 'extra')
        if (any(descriptor[k] != getattr(texture, k) for k in fields)
                or descriptor['format'] != texture.dxgi
                or descriptor['asset_key_at'] != texture.asset_key):
            raise ValueError('Python analysis and native descriptor disagree')
        if len(analysis['available']) < count or not all(analysis['available'][:count]):
            raise UnsupportedError('not every requested mip is available in the analysis')
        return self.store.texture_dds(texture, mips=count)

    def texture_dds(self, pool, analysis, mips=None, *, allow_provisional=None, history=None):
        """Bytes-returning counterpart using the backend's fixed decode policy."""
        return self.export(pool, analysis, mips, allow_provisional=allow_provisional,
                           history=history).data

    def close(self):
        self.store.close()

    def __enter__(self):
        self.store.__enter__()
        return self

    def __exit__(self, *_):
        self.close()

