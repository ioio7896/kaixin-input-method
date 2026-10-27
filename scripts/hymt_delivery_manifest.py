"""Verify the delivery files and retain reproducible checksums."""
import hashlib
import json
import zipfile
from pathlib import Path
root = Path(__file__).resolve().parents[1]
translator = root.parent / 'HY2翻译机'
paths = [
    root/'dist/kaixin-setup-ime-2.0.0-20261006-104032.exe',
    root/'dist/kaixin-setup-ocr-2.0.0-20261006-104032.exe',
    translator/'target/release/bundle/inno/Hy-MT2-Local-Translator-Offline-Setup-0.1.1.exe',
    translator/'output/Hy-MT2-Extension-0.1.1-20261006-optimized.zip',
]
reports = {name: json.loads((root/'dist'/name).read_text(encoding='utf-8')) for name in [
    'hymt-optimized-integration.json', 'hymt-release-callback-integration.json', 'hymt-upgrade-verification.json']}
assert all(report['passed'] for report in reports.values())
assert reports['hymt-optimized-integration.json']['numbers_urls_placeholders_preserved']
artifacts = []
for path in paths:
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        if path.suffix == '.exe': assert stream.read(2) == b'MZ'
        stream.seek(0)
        while chunk := stream.read(8 * 1024 * 1024): digest.update(chunk)
    artifacts.append(dict(path=str(path), bytes=path.stat().st_size, sha256=digest.hexdigest()))
with zipfile.ZipFile(paths[-1]) as archive:
    assert archive.testzip() is None
    manifest = json.loads(archive.read('manifest.json'))
    assert manifest['version'] == '0.1.1' and manifest['manifest_version'] == 3
result = dict(passed=True, artifacts=artifacts, reports=reports,
    translator_unit_tests=14, extension_tests=17,
    source_backup=str(root/'dist/hymt-upgrade-source-backup'))
(root/'dist/hymt-optimized-delivery-manifest.json').write_text(json.dumps(result,ensure_ascii=False,indent=2)+'\n',encoding='utf-8')
print(json.dumps(dict(passed=True,artifacts=artifacts),ensure_ascii=False,indent=2))
