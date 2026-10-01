"""Exercise staged engine IPC and all OCR profiles without installing them."""
import json,os,subprocess,sys,uuid
from pathlib import Path
ROOT=Path(__file__).resolve().parents[1]
OUT=ROOT/'dist/unification-verification'
OUT.mkdir(parents=True,exist_ok=True)
def run(command,env=None):
 result=subprocess.run(list(map(str,command)),env=env,capture_output=True,encoding='utf-8',errors='replace',timeout=120)
 if result.returncode: raise RuntimeError(result.stdout+'\n'+result.stderr)
 return result.stdout
package=ROOT/'dist/kaixin-package-ime'
env={k.upper():v for k,v in os.environ.items()}
env['PYTHONDONTWRITEBYTECODE']='1'
env['LOCALAPPDATA']=str(OUT/'isolated-user');env['APPDATA']=env['LOCALAPPDATA']
env['SRF_ENGINE_PIPE_NAME']=r'\\.\pipe\kaixin-package-check-'+uuid.uuid4().hex
env['SRF_ENGINE_MUTEX_NAME']='Local\\kaixin-package-check-'+uuid.uuid4().hex
fixture=Path(env['LOCALAPPDATA'])/'kaixin'
fixture.mkdir(parents=True,exist_ok=True)
(fixture/'engine_capability.dat').write_text(uuid.uuid4().hex,encoding='utf-8')
engine=[]
for reading in ('nihao','xiong','zhrmghg'):
 engine.append(run([package/'srf_ime_engine.exe','--install-health-check','--probe',reading,'--lexicon-dir',package/'lexicon'],env))
(OUT/'engine-probes.json').write_text(json.dumps(engine,ensure_ascii=False,indent=2),encoding='utf-8')
package=ROOT/'dist/kaixin-package-ocr'
helper=package/'tools/kaixin_ocr_engine.py';python=package/'.python-runtime/python.exe'
bootstrap="import sys,runpy;sys.path.insert(0,sys.argv.pop(1));runpy.run_path(sys.argv.pop(1),run_name='__main__')"
for profile in ('fast','balanced','accurate'):
 raw=run([python,'-B','-c',bootstrap,package/'.python-packages',helper,ROOT/'RapidOCR-3.9.0/python/tests/test_files/ch_en_num.jpg','--rapidocr-root',package/'RapidOCR-3.9.0','--provider','cpu','--profile',profile,'--vis',OUT/f'ocr-{profile}.jpg'],env)
 payload=json.loads(raw)
 if not payload.get('ok'): raise RuntimeError(raw)
 if not (OUT/f'ocr-{profile}.jpg').is_file(): raise RuntimeError('OCR visualization missing: '+raw)
 if not payload.get('text','').strip(): raise RuntimeError('OCR returned no text: '+raw)
 (OUT/f'ocr-{profile}.json').write_text(json.dumps(payload,ensure_ascii=False,indent=2),encoding='utf-8')
 print(profile, 'OCR passed')
print('Staged engine IPC and OCR profiles passed')
