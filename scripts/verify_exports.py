"""Compare complete DDS bytes against an external Leonida checkout, locally."""
import argparse
import hashlib
import importlib
import json
import pathlib
import subprocess
import sys
import tempfile


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--leonida',type=pathlib.Path,required=True)
    p.add_argument('--stores',type=pathlib.Path,required=True)
    p.add_argument('--exe',type=pathlib.Path,required=True)
    p.add_argument('--out',type=pathlib.Path,required=True)
    p.add_argument('--per-store',type=int,default=12)
    args=p.parse_args()
    if not 1<=args.per_store<=1000:p.error('invalid per-store limit')
    if args.out.resolve().is_relative_to(pathlib.Path(__file__).resolve().parents[1]):p.error('report must be outside repository')
    sys.path.insert(0,str(args.leonida.resolve()/'tools'));baseline=importlib.import_module('ttpf_mips')
    result=dict(stores=0,equal=0,baseline_unavailable=0,errors=[],groups={},cases=[],executable_sha256=hashlib.sha256(args.exe.read_bytes()).hexdigest())
    paths=[args.stores] if args.stores.is_file() else sorted(args.stores.rglob('*.ttpf'))
    for path in paths:
        native=json.loads(subprocess.check_output([str(args.exe),'list',str(path)]))
        pool=baseline.Pool(str(path))
        try:
            if len(native)!=pool.textures:raise ValueError('catalog count mismatch')
            for t in native:
                d=pool.descriptor(t['index'])
                mapping={'width':'width','height':'height','depth':'depth','mips':'mips','dxgi':'format','dimension':'dimension','mode':'mode','non_tail':'non_tail','base':'base','meta':'meta','extra':'extra','asset_key':'asset_key_at'}
                if any(t[a]!=d[b] for a,b in mapping.items()):raise ValueError('catalog descriptor mismatch')
            # Grouped sampling covers shape, format and tile layout, not just first entries.
            groups={}
            for t in native:
                key=(t['dimension'],t['depth']>1,t['dxgi'],t['mode'])
                groups.setdefault(key,[]).append(t)
            chosen=[]
            for group in groups.values():
                chosen.append(group[len(group)//2])
            chosen=chosen[:args.per_store]
            for t in chosen:
                try:
                    a=baseline.analyze(pool,t['index'],allow_bcpack=False)
                    expected=baseline.texture_dds(pool,a)
                except (ValueError,OSError):
                    result['baseline_unavailable']+=1;continue
                with tempfile.TemporaryDirectory(prefix='rage-export-') as temp:
                    dest=pathlib.Path(temp)/'texture.dds'
                    run=subprocess.run([str(args.exe),'export',str(path),str(t['index']),str(dest)],capture_output=True,text=True)
                    if run.returncode:
                        result['errors'].append(dict(store=path.name,index=t['index'],error=run.stderr.strip()));continue
                    actual=dest.read_bytes()
                    if actual!=expected:
                        result['errors'].append(dict(store=path.name,index=t['index'],error='DDS bytes differ',header_equal=actual[:148]==expected[:148],payload_equal=actual[148:]==expected[148:]));continue
                result['equal']+=1
                result['cases'].append(dict(store=str(path.resolve()),index=t['index'],source_size=path.stat().st_size,source_mtime_ns=path.stat().st_mtime_ns,dds_sha256=hashlib.sha256(expected).hexdigest()))
                group=f"dimension={t['dimension']} depth={t['depth']} dxgi={t['dxgi']} mode={t['mode']}"
                result['groups'][group]=result['groups'].get(group,0)+1
            result['stores']+=1
            print(f"checked {result['stores']}/{len(paths)} stores: {result['equal']} equal, {len(result['errors'])} errors",file=sys.stderr,flush=True)
        finally:pool.close()
    if not paths:raise ValueError('no stores found')
    if hashlib.sha256(args.exe.read_bytes()).hexdigest()!=result['executable_sha256']:raise ValueError('native binary changed during validation')
    args.out.parent.mkdir(parents=True,exist_ok=True);args.out.write_text(json.dumps(result,indent=2),encoding='utf-8');print(json.dumps({k:v for k,v in result.items() if k not in ('cases','groups')},indent=2))
    return bool(result['errors']) or result['equal']==0


if __name__=='__main__':sys.exit(main())
