"""Compare every requested mip/slice against a separately built SixNG core.

Sampling is grouped by typed format, dimension and array presence. Missing
subresources and metadata disagreements remain failures, not skipped passes.
No decoded pixels are retained. Native decoded-cache budget must be zero.
"""
import argparse
import hashlib
import json
import pathlib
import subprocess


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--store', type=pathlib.Path, action='append', required=True)
    p.add_argument('--catalog', type=pathlib.Path, required=True)
    p.add_argument('--native', type=pathlib.Path, required=True)
    p.add_argument('--reference', type=pathlib.Path, required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    p.add_argument('--index', type=int, action='append')
    p.add_argument('--limit', type=int, default=4096)
    a = p.parse_args()
    if a.out.resolve().is_relative_to(pathlib.Path(__file__).resolve().parents[1]):
        p.error('reports must stay outside repository')
    if not 1 <= a.limit <= 16384:p.error('invalid subresource limit')
    binaries = [a.catalog, a.native, a.reference]
    fingerprints = [hashlib.sha256(x.read_bytes()).hexdigest() for x in binaries]
    report = dict(scope=__doc__, binaries_sha256=fingerprints, textures=0,
                  equal=0, errors=[], metadata_errors=[], attempted=0, truncated=False)

    def call(args):
        r = subprocess.run([str(x) for x in args], text=True, capture_output=True)
        if r.returncode:raise ValueError(r.stderr.strip()[-500:])
        return json.loads(r.stdout)

    for store in a.store:
        stat = store.stat()
        catalog = call([a.catalog, 'list', store])
        if a.index:
            textures = [catalog[i] for i in a.index]
        else:
            groups = {}
            for t in catalog:
                if t['asset_key'] is not None:
                    groups.setdefault((t['dxgi'],t['dimension'],t['depth'] > 1), []).append(t)
            textures = [v[len(v)//2] for v in groups.values()]
        for t in textures:
            report['textures'] += 1
            if t['asset_key'] is None:
                report['metadata_errors'].append(dict(store=store.name,index=t['index'],error='keyless descriptor'))
                continue
            checked_metadata = False
            for mip in range(t['mips']):
                slices = max(1, t['depth'] >> mip) if t['dimension'] == 3 else t['depth']
                for sl in range(slices):
                    if report['attempted'] == a.limit:
                        report['truncated'] = True
                        break
                    report['attempted'] += 1
                    case = dict(store=store.name,index=t['index'],dxgi=t['dxgi'],mip=mip,slice=sl)
                    try:
                        native = call([a.native,store,t['index'],mip,sl])
                        stats = native['stats']
                        if any(stats[k] for k in ('cache_hits','cache_bytes','peak_cache_bytes','evictions')):
                            raise ValueError('native decoded cache was used')
                        reference = call([a.reference,'image',store,f"{t['asset_key']:016x}",mip,sl])
                        if not checked_metadata:
                            checked_metadata = True
                            if reference['texture']['mips'] != t['mips']:
                                report['metadata_errors'].append(dict(store=store.name,index=t['index'],
                                    native_mips=t['mips'],sixng_mips=reference['texture']['mips']))
                        if reference['skipped']:
                            raise ValueError('SixNG skipped input data')
                        if (native['bytes'],native['sha256']) != (reference['bytes'],reference['sha256']):
                            raise ValueError('preview pixels differ')
                        report['equal'] += 1
                    except ValueError as e:
                        report['errors'].append(dict(**case,error=str(e)))
                if report['truncated']:break
            if report['truncated']:break
        if (store.stat().st_size,store.stat().st_mtime_ns) != (stat.st_size,stat.st_mtime_ns):
            raise ValueError('source changed during validation')
        if report['truncated']:break
    if fingerprints != [hashlib.sha256(x.read_bytes()).hexdigest() for x in binaries]:
        raise ValueError('binary changed during validation')
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(json.dumps(dict(textures=report['textures'],attempted=report['attempted'],equal=report['equal'],
                         pixel_errors=len(report['errors']),metadata_errors=report['metadata_errors'],
                         truncated=report['truncated']),indent=2))
    return bool(report['errors'] or report['metadata_errors'] or report['truncated']) or not report['equal']


if __name__ == '__main__':
    raise SystemExit(main())
