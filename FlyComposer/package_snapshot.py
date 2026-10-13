"""Package the app and one library snapshot, without local environments or models."""
from pathlib import Path
import json
import os
import zipfile


def package():
    root=Path(__file__).resolve().parent
    index_bytes=(root/'library/index.json').read_bytes()
    index=json.loads(index_bytes)
    files=set()
    for name in ('bin','data','flycomposer','profiles','tests','web'):
        for path in (root/name).rglob('*'):
            if path.is_file() and '__pycache__' not in path.parts and not (name=='data' and 'raw' in path.relative_to(root).parts):
                files.add(path)
    files.update(p for p in root.iterdir() if p.is_file() and p.suffix in {'.py','.md','.bat','.txt'})
    for track in index['tracks']:
        files.update(p for p in (root/'library'/track['id']).iterdir() if p.is_file())
    for path in (root/'reports').iterdir():
        if path.is_file() and path.suffix in {'.json','.md','.txt'} and not path.name.startswith('pre-'):
            files.add(path)
        elif path.is_dir() and path.name.startswith('organist-'):
            files.update(p for p in path.rglob('*') if p.is_file())
    pending=root/'FlyComposer-standalone.next.zip'
    with zipfile.ZipFile(pending,'w',zipfile.ZIP_DEFLATED,compresslevel=2) as archive:
        for path in sorted(files):
            archive.write(path,str(Path('FlyComposer')/path.relative_to(root)))
        archive.writestr('FlyComposer/library/index.json',index_bytes)
    with zipfile.ZipFile(pending) as archive:
        assert archive.read('FlyComposer/library/index.json')==index_bytes
        assert archive.testzip() is None
    destination=root/'FlyComposer-standalone.zip'
    os.replace(pending,destination)
    result={'archive':str(destination),'tracks':len(index['tracks']),'files':len(files)+1,
            'bytes':destination.stat().st_size,'crc_verified':True}
    print(json.dumps(result))
    return result


if __name__=='__main__':
    package()
