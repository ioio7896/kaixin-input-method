"""Lossless indexed container for the standard and hot prebaked profiles."""
import argparse, struct, zlib
from pathlib import Path
MAGIC=b'KXLP0001'
def read_pack(path):
 data=Path(path).read_bytes()
 if len(data)<40 or data[:8]!=MAGIC:raise ValueError('Invalid lexicon pack header')
 a,b,ua,ub=struct.unpack_from('<4Q',data,8)
 if a+b+40!=len(data) or max(ua,ub)>512*1024*1024:raise ValueError('Invalid lexicon pack bounds')
 results=[]
 for payload,length in [(data[40:40+a],ua),(data[40+a:],ub)]:
  decoder=zlib.decompressobj();raw=decoder.decompress(payload,length+1)
  if len(raw)!=length or not decoder.eof or decoder.unused_data or raw[:8]!=b'SRFLX002':raise ValueError('Invalid lexicon profile')
  results.append(raw)
 return results

def pack(directory):
 root=Path(directory).resolve();paths=[root/'lexicon.bin',root/'hot_lexicon.bin']
 source=[p.read_bytes() for p in paths];compressed=[zlib.compress(data,9) for data in source]
 data=MAGIC+struct.pack('<4Q',len(compressed[0]),len(compressed[1]),len(source[0]),len(source[1]))+b''.join(compressed)
 target=root/'lexicon.pack';temp=target.with_suffix('.tmp');temp.write_bytes(data);temp.replace(target)
 if read_pack(target)!=source:raise ValueError('Lexicon pack round-trip failed')
 for p in paths:p.unlink()
 print(f'Lossless lexicon pack: {sum(map(len,source)):,} -> {len(data):,} bytes')
if __name__=='__main__':
 parser=argparse.ArgumentParser();parser.add_argument('directory');args=parser.parse_args();pack(args.directory)
