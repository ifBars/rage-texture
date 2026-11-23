"""Check real BC6H block-to-half decoding against external bcdec.

This validates the pixel decoder after detiling. It does not confirm BCPack
initial histories, even when --allow-fitted supplies reconstructed blocks.
All temporary blocks and pixel outputs remain outside the repository.
"""
import argparse
import hashlib
import json
import pathlib
import subprocess
import tempfile


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--store', type=pathlib.Path, action='append', required=True)
    p.add_argument('--native', type=pathlib.Path, required=True)
    p.add_argument('--reference', type=pathlib.Path, required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    p.add_argument('--per-format', type=int, default=8)
    p.add_argument('--allow-fitted', action='store_true')
    a = p.parse_args()
    if a.out.resolve().is_relative_to(pathlib.Path(__file__).resolve().parents[1]):
        p.error('reports must remain outside repository')
    if not 1 <= a.per_format <= 128:p.error('invalid sample limit')
    from rage_texture import Store, UnsupportedError
    binaries = [a.native, a.reference]
    fingerprints = [hashlib.sha256(x.read_bytes()).hexdigest() for x in binaries]
    report = dict(scope=__doc__, native_sha256=fingerprints[0], reference_sha256=fingerprints[1],
                  reference_revision='80859ed3b7afb1c527a2a99d70c61457bea72d0c',
                  cases=[], unavailable=[], half_channels=0, blocks=0, errors=[])
    with tempfile.TemporaryDirectory(prefix='rage-real-bc6h-') as folder:
        folder = pathlib.Path(folder)
        for path in a.store:
            with Store(path, cache_bytes=0, allow_fitted=a.allow_fitted) as store:
                catalog = store.textures()
                for dxgi in (95, 96):
                    group = [t for t in catalog if t.dxgi == dxgi]
                    count = min(a.per_format, len(group))
                    for i in range(count):
                        t = group[i*len(group)//count]
                        case = dict(store=path.name, index=t.index, signed=dxgi == 96)
                        try:
                            exported = store.texture_mip(t, 0, 0)
                        except UnsupportedError as e:
                            report['unavailable'].append(dict(**case, reason=str(e)))
                            continue
                        data = exported.data
                        if len(data) > 16*1_000_000:
                            report['unavailable'].append(dict(**case, reason='test helper block limit'))
                            continue
                        source, native, reference = [folder/n for n in ('blocks.bin','native.bin','reference.bin')]
                        source.write_bytes(data)
                        subprocess.run([str(a.native), str(source), str(native), str(int(dxgi == 96))],
                                       check=True, capture_output=True)
                        subprocess.run([str(a.reference), str(source), str(reference), str(int(dxgi == 96))],
                                       check=True, capture_output=True)
                        left, right = native.read_bytes(), reference.read_bytes()
                        if len(left) != len(data)//16*96 or left != right:
                            report['errors'].append(dict(**case, reason='half-value mismatch'))
                            continue
                        case.update(blocks=len(data)//16, half_channels=len(left)//2, equal=True,
                                    lossless_container=all(q.is_lossless for q in exported.quality))
                        report['cases'].append(case)
                        report['blocks'] += case['blocks']
                        report['half_channels'] += case['half_channels']
                stats = store.stats()
                if stats.cache_hits or stats.cache_bytes or stats.peak_cache_bytes:
                    raise ValueError('decoded cache was used')
    if fingerprints != [hashlib.sha256(x.read_bytes()).hexdigest() for x in binaries]:
        raise ValueError('oracle binary changed during validation')
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(json.dumps(dict(cases=len(report['cases']), blocks=report['blocks'],
                         half_channels=report['half_channels'], unavailable=len(report['unavailable']),
                         errors=report['errors']), indent=2))
    return bool(report['errors']) or not report['cases']


if __name__ == '__main__':
    raise SystemExit(main())
