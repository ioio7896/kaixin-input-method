"""Trim verified developer-only assets inside a staged package."""
import argparse,json,shutil
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
def trim(package):
 root=Path(package).resolve();dist=(ROOT/'dist').resolve()
 if dist not in root.parents or root.name not in ['kaixin-package-ime','kaixin-package-ocr']:raise ValueError('Expected a staged package inside workspace dist')
 policy=json.loads((ROOT/'shared/package_trim.json').read_text());removed=0
 for rel in policy['directories']:
  p=root/rel
  if not p.exists():continue
  if p.is_symlink() or root not in p.resolve().parents:raise ValueError('Unsafe package path')
  files=list(p.rglob('*'))
  if any(f.is_symlink() for f in files):raise ValueError('Reparse link in package trim target')
  removed+=sum(f.stat().st_size for f in files if f.is_file());shutil.rmtree(p)
 for pattern in policy['file_globs']:
  for p in root.glob(pattern):
   if p.is_symlink() or root not in p.resolve().parents:raise ValueError('Unsafe package file')
   removed+=p.stat().st_size;p.unlink()
 print(f'Package trim removed {removed:,} developer-asset bytes')
if __name__=='__main__':
 p=argparse.ArgumentParser();p.add_argument('package');trim(p.parse_args().package)
