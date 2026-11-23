"""Verify the installed Python SDK against saved complete DDS reference hashes."""
import argparse
import hashlib
import json
import pathlib


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--reference-report', type=pathlib.Path, action='append', required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    a = p.parse_args()
    if a.out.resolve().is_relative_to(pathlib.Path(__file__).resolve().parents[1]):
        p.error('raw reports must remain outside repository')
    import rage_texture._native as native
    from rage_texture import Store
    module = pathlib.Path(native.__file__)
    fingerprint = hashlib.sha256(module.read_bytes()).hexdigest()
    groups = {}
    for report in a.reference_report:
        for case in json.loads(report.read_text(encoding='utf-8'))['cases']:
            groups.setdefault(case['store'], []).append(case)
    result = dict(equal=0, errors=[], native_extension_sha256=fingerprint)
    for path, cases in groups.items():
        try:
            stat = pathlib.Path(path).stat()
            if any(c['source_size'] != stat.st_size or c['source_mtime_ns'] != stat.st_mtime_ns for c in cases):
                raise ValueError('reference source changed')
            with Store(path, cache_bytes=0, allow_fitted=False) as store:
                catalog = store.textures()
                for case in cases:
                    exported = store.texture_dds(catalog[case['index']])
                    if hashlib.sha256(exported.data).hexdigest() != case['dds_sha256']:
                        raise ValueError(f"DDS mismatch at {case['index']}")
                    if any(not q.is_lossless for q in exported.quality):
                        raise ValueError('unexpected fitted reconstruction')
                    stats = store.stats()
                    if any(getattr(stats,k) for k in ('cache_hits','cache_bytes','peak_cache_bytes','evictions')):
                        raise ValueError('decoded cache was used')
                    result['equal'] += 1
        except Exception as e:
            result['errors'].append(dict(store=pathlib.Path(path).name, error=str(e)))
    if hashlib.sha256(module.read_bytes()).hexdigest() != fingerprint:
        raise ValueError('native extension changed during validation')
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text(json.dumps(result,indent=2),encoding='utf-8')
    print(json.dumps(result,indent=2))
    return bool(result['errors']) or not result['equal']


if __name__ == '__main__':
    raise SystemExit(main())
