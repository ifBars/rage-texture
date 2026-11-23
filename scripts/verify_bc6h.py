"""Compare synthetic BC6H half bits against a separate external bcdec C oracle."""
import argparse
import hashlib
import json
import pathlib
import random
import subprocess
import tempfile


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--native', type=pathlib.Path, required=True)
    p.add_argument('--reference', type=pathlib.Path, required=True)
    p.add_argument('--out', type=pathlib.Path, required=True)
    p.add_argument('--per-mode', type=int, default=4096)
    a = p.parse_args()
    if a.out.resolve().is_relative_to(pathlib.Path(__file__).resolve().parents[1]):
        p.error('raw reports must remain outside repository')
    if not 1 <= a.per_mode <= 32768:
        p.error('per-mode must be 1..32768')
    rng = random.Random(0xBC6_2026)
    # 14 valid mode codes plus 4 reserved ones, with randomized partitions and
    # endpoint/index bits. Reference tables/decoding are never copied here.
    modes = [0, 1, 2, 6, 10, 14, 18, 22, 26, 30, 3, 7, 11, 15, 19, 23, 27, 31]
    blocks = bytearray()
    for mode in modes:
        for _ in range(a.per_mode):
            block = bytearray(rng.randbytes(16))
            mask = 3 if mode < 2 else 31
            block[0] = (block[0] & ~mask) | mode
            blocks.extend(block)
    binaries = [a.native, a.reference]
    before = [hashlib.sha256(x.read_bytes()).hexdigest() for x in binaries]
    results = []
    with tempfile.TemporaryDirectory(prefix='rage-bc6h-') as temp:
        temp = pathlib.Path(temp)
        source = temp / 'synthetic.bin'
        source.write_bytes(blocks)
        for signed in (0, 1):
            native, reference = temp/'native.bin', temp/'reference.bin'
            meta = json.loads(subprocess.check_output([str(a.native), str(source), str(native), str(signed)]))
            subprocess.run([str(a.reference), str(source), str(reference), str(signed)], check=True)
            x, y = native.read_bytes(), reference.read_bytes()
            mismatches = sum(x[i:i+2] != y[i:i+2] for i in range(0, min(len(x), len(y)), 2))
            if len(x) != len(y) or mismatches:
                details = []
                for i in range(0, min(len(x), len(y)), 2):
                    if x[i:i+2] != y[i:i+2]:
                        block_index = i // 96
                        details.append(dict(block_index=block_index, mode_code=modes[block_index//a.per_mode],
                                            block=blocks[block_index*16:block_index*16+16].hex(),
                                            channel=i%96//2, native=int.from_bytes(x[i:i+2],'little'),
                                            reference=int.from_bytes(y[i:i+2],'little')))
                        if len(details) == 32:break
                a.out.parent.mkdir(parents=True, exist_ok=True)
                a.out.write_text(json.dumps(dict(signed=signed,mismatches=mismatches,details=details),indent=2),encoding='utf-8')
                raise ValueError(f'half-bit mismatch signed={signed}, channels={mismatches}, lengths={len(x)},{len(y)}')
            results.append(dict(signed=bool(signed), half_channels=len(x)//2, equal=True, **meta))
    if before != [hashlib.sha256(x.read_bytes()).hexdigest() for x in binaries]:
        raise ValueError('binary changed during validation')
    report = dict(reference_revision='80859ed3b7afb1c527a2a99d70c61457bea72d0c',
                  native_sha256=before[0], reference_sha256=before[1],
                  per_mode=a.per_mode, results=results)
    a.out.parent.mkdir(parents=True, exist_ok=True)
    a.out.write_text(json.dumps(report, indent=2), encoding='utf-8')
    print(json.dumps(report, indent=2))


if __name__ == '__main__':
    main()
