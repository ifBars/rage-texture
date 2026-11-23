"""Compare native chunk bytes and preview pixels with unmodified SixNG core."""
import argparse
import json
import pathlib
import subprocess
import sys


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--store',type=pathlib.Path,required=True);p.add_argument('--exe',type=pathlib.Path,required=True)
    p.add_argument('--reference',type=pathlib.Path,required=True);p.add_argument('--out',type=pathlib.Path,required=True)
    p.add_argument('--per-codec',type=int,default=8);p.add_argument('--previews',type=int,default=16)
    a=p.parse_args()
    if not 1 <= a.per_codec <= 4096 or not 0 <= a.previews <= 256:p.error('invalid sample limits')
    if a.out.resolve().is_relative_to(pathlib.Path(__file__).resolve().parents[1]):p.error('report must be outside repository')
    def call(args):
        r=subprocess.run([str(s) for s in args],capture_output=True,text=True)
        if r.returncode:raise ValueError(r.stderr.strip()[-500:])
        return json.loads(r.stdout)
    result=dict(chunk_equal={},chunk_errors=[],preview_equal=0,preview_errors=[])
    meta=call([a.exe,'inspect',a.store])
    for codec in [0,1,3,4,5,6,7,8]:
        group=[c for c in meta['chunks'] if c['codec']==codec];n=min(a.per_codec,len(group))
        for i in range(n):
            c=group[i*len(group)//n]
            try:
                args=[a.exe,'range-hash',a.store,c['virtual_offset'],c['decoded_bytes']]
                if codec not in (0,8):args+=['--allow-fitted']
                native=call(args);ref=call([a.reference,'range',a.store,c['virtual_offset'],c['decoded_bytes']])
                if ref['skipped'] or native['sha256']!=ref['sha256'] or native['bytes']!=ref['bytes']:raise ValueError('decoded bytes differ or reference skipped data')
                result['chunk_equal'][str(codec)]=result['chunk_equal'].get(str(codec),0)+1
            except ValueError as e:result['chunk_errors'].append(dict(codec=codec,sample=i,error=str(e)))
    textures=call([a.exe,'list',a.store]);groups={}
    for t in textures:
        if t['asset_key'] is not None:groups.setdefault((t['dxgi'],t['dimension'],t['depth']>1),[]).append(t)
    for group in list(groups.values())[:a.previews]:
        t=group[len(group)//2]
        try:
            native=call([a.exe,'mip-hash',a.store,t['index'],0,0,'--allow-fitted']);ref=call([a.reference,'image',a.store,f"{t['asset_key']:016x}",0,0])
            if ref['skipped'] or native['sha256']!=ref['sha256'] or native['bytes']!=ref['bytes']:raise ValueError('preview pixels differ or reference skipped data')
            result['preview_equal']+=1
        except ValueError as e:result['preview_errors'].append(dict(index=t['index'],dxgi=t['dxgi'],dimension=t['dimension'],error=str(e)))
    a.out.parent.mkdir(parents=True,exist_ok=True);a.out.write_text(json.dumps(result,indent=2),encoding='utf-8');print(json.dumps(result,indent=2))
    return bool(result['chunk_errors'] or result['preview_errors']) or not result['chunk_equal']


if __name__=='__main__':sys.exit(main())
