"""Check wheel structure and keep generated/game data out of source packages."""
import argparse
import pathlib
import tarfile
import zipfile


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--wheel', type=pathlib.Path)
    p.add_argument('--sdist', type=pathlib.Path)
    p.add_argument('--directory', type=pathlib.Path)
    a = p.parse_args()
    if a.directory is not None:
        if a.wheel is not None or a.sdist is not None:p.error('choose directory or explicit artifact paths')
        wheels, sources = list(a.directory.glob('*.whl')), list(a.directory.glob('*.tar.gz'))
        if len(wheels) != 1 or len(sources) != 1:p.error('directory must contain exactly one wheel and one sdist')
        a.wheel, a.sdist = wheels[0], sources[0]
    elif a.wheel is None or a.sdist is None:
        p.error('supply --directory or both --wheel and --sdist')
    with zipfile.ZipFile(a.wheel) as wheel:
        names = wheel.namelist()
        for name in names:
            top = pathlib.PurePosixPath(name).parts[0]
            if top != 'rage_texture' and not top.endswith('.dist-info'):
                raise ValueError('wheel would install an unrelated top-level file: '+name)
        for required in ('rage_texture/_native.pyi', 'rage_texture/py.typed',
                         'rage_texture/THIRD_PARTY_NOTICES.md'):
            if required not in names:raise ValueError('wheel missing '+required)
        if not any('/licenses/LICENSE' in n for n in names):
            raise ValueError('wheel missing GPL license metadata')
        notices = wheel.read('rage_texture/THIRD_PARTY_NOTICES.md').decode('utf-8')
        if 'PyO3' not in notices or '2979c57f' not in notices:
            raise ValueError('wheel notices lack binding/decoder attribution')
    with tarfile.open(a.sdist) as source:
        for member in source.getmembers():
            path = pathlib.PurePosixPath(member.name)
            if path.is_absolute() or '..' in path.parts or member.issym() or member.islnk():
                raise ValueError('unsafe source archive path: '+member.name)
            if any(part in ('target', 'dist', '.git', '.venv', '__pycache__') for part in path.parts):
                raise ValueError('generated source archive member: '+member.name)
            if path.suffix.lower() in ('.ttpf','.rpf','.bin','.dds','.png','.whl','.pyd','.dll','.exe','.so','.dylib'):
                raise ValueError('binary/game output in source archive: '+member.name)
    print('wheel structure and source archive checks passed')


if __name__ == '__main__':
    main()
