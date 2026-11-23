"""Reuse saved reference hashes only while the source file identity stamp is unchanged."""
import argparse
import hashlib
import json
import pathlib
import subprocess
import tempfile


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--reference-report',type=pathlib.Path,action='append',required=True)
    p.add_argument('--exe',type=pathlib.Path,required=True);p.add_argument('--out',type=pathlib.Path,required=True);a=p.parse_args()
    if a.out.resolve().is_relative_to(pathlib.Path(__file__).resolve().parents[1]):p.error('reports must stay outside repository')
    result=dict(equal=0,errors=[],executable_sha256=hashlib.sha256(a.exe.read_bytes()).hexdigest())
    for report in a.reference_report:
        for case in json.loads(report.read_text(encoding='utf-8'))['cases']:
            path=pathlib.Path(case['store']);stamp=path.stat()
            if (stamp.st_size,stamp.st_mtime_ns)!=(case['source_size'],case['source_mtime_ns']):raise ValueError('source changed; regenerate reference outputs')
            with tempfile.TemporaryDirectory(prefix='rage-recheck-') as folder:
                dest=pathlib.Path(folder)/'output.dds';r=subprocess.run([str(a.exe),'export',str(path),str(case['index']),str(dest)],capture_output=True,text=True)
                if r.returncode or hashlib.sha256(dest.read_bytes()).hexdigest()!=case['dds_sha256']:
                    result['errors'].append(dict(store=path.name,index=case['index'],error=r.stderr.strip() or 'DDS bytes disagree'))
                else:result['equal']+=1
    if hashlib.sha256(a.exe.read_bytes()).hexdigest()!=result['executable_sha256']:raise ValueError('binary changed during check')
    a.out.parent.mkdir(parents=True,exist_ok=True);a.out.write_text(json.dumps(result,indent=2),encoding='utf-8');print(json.dumps(result,indent=2))
    return bool(result['errors'])


if __name__=='__main__':raise SystemExit(main())
