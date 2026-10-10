"""Regression checks for the IME side of the HY2 integration."""
import json
import subprocess
import time
from pathlib import Path
root = Path(__file__).resolve().parents[1]
checks = [
    ['cargo', 'test', '-p', 'kaixin-core', '--lib'],
    ['cargo', 'test', '-p', 'kaixin-common', '--lib'],
    ['cargo', 'test', '-p', 'pinyin-ime', '--bin', 'srf_ime_settings'],
    ['ctest', '--test-dir', 'tsf-tip/build-package', '-C', 'Release', '--output-on-failure'],
    ['ctest', '--test-dir', 'tsf-tip/build-package-x86', '-C', 'Release', '--output-on-failure'],
    ['python', 'scripts/generate_shared_contracts.py', '--check'],
    ['python', 'scripts/check_package_manifest.py'],
]
report = []
with (root/'dist/hymt-upgrade-verification.log').open('w', encoding='utf-8') as log:
    for command in checks:
        started = time.monotonic()
        log.write('\n' + ' '.join(command) + '\n'); log.flush()
        result = subprocess.run(command, cwd=root, stdout=log, stderr=subprocess.STDOUT)
        report.append(dict(command=command, exit_code=result.returncode, seconds=round(time.monotonic()-started, 2)))
        if result.returncode:
            (root/'dist/hymt-upgrade-verification.json').write_text(json.dumps(dict(passed=False, checks=report), indent=2), encoding='utf-8')
            raise SystemExit(result.returncode)
(root/'dist/hymt-upgrade-verification.json').write_text(json.dumps(dict(passed=True, checks=report), indent=2), encoding='utf-8')
print('All IME regression and package checks passed')
