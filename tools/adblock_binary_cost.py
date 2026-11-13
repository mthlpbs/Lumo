"""Compare PE function bytes by demangled Rust crate, retaining nm's totals.

COFF nm reports zero sizes with our LLVM toolchain; x64 PE unwind ranges
provide a useful function-code fallback. Data and inlined origins are excluded.
"""
import argparse
from collections import defaultdict
import json
from pathlib import Path
import re
import struct

def measure(root):
    raw=(root/'symbols.txt').read_bytes()
    text=raw.decode('utf-16') if raw.startswith(b'\xff\xfe') else raw.decode('utf-8-sig')
    names,reported={},defaultdict(int)
    def owner(symbol):
        name=re.search(r'(?:^|<)([a-zA-Z_][a-zA-Z_0-9]*)::',symbol)
        return name.group(1) if name else 'native_or_other'
    for line in text.splitlines():
        fields=line.split(maxsplit=3)
        if len(fields)!=4:continue
        address,size,kind,symbol=fields
        reported[owner(symbol)]+=int(size,16)
        if kind in ('T','t'):names.setdefault(int(address,16),symbol)
    data=(root/'lumo-unstripped.exe').read_bytes()
    pe=struct.unpack_from('<I',data,0x3c)[0]
    count=struct.unpack_from('<H',data,pe+6)[0]
    opt_size=struct.unpack_from('<H',data,pe+20)[0]
    opt=pe+24
    image_base=struct.unpack_from('<Q',data,opt+24)[0]
    exception_rva,exception_size=struct.unpack_from('<II',data,opt+112+3*8)
    sections=[]
    for i in range(count):
        size,rva,raw_size,offset=struct.unpack_from('<IIII',data,opt+opt_size+i*40+8)
        sections.append((rva,max(size,raw_size),offset))
    def at(rva):
        for start,size,offset in sections:
            if start<=rva<start+size:return offset+rva-start
        raise ValueError('PE RVA outside sections')
    code=defaultdict(int)
    start=at(exception_rva)
    for offset in range(start,start+exception_size,12):
        begin,end,_=struct.unpack_from('<III',data,offset)
        symbol=names.get(image_base+begin)
        code[owner(symbol) if symbol else 'unattributed']+=end-begin
    return dict(unstripped_bytes=len(data),nm_reported_sizes=dict(reported),
                unwind_function_bytes=dict(sorted(code.items(),key=lambda x:-x[1])))

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--before',type=Path,required=True)
    parser.add_argument('--after',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    report={'before':measure(args.before),'after':measure(args.after)}
    args.output.write_text(json.dumps(report,indent=2)+'\n',encoding='utf-8')
    print(json.dumps(report,indent=2))

if __name__=='__main__':main()
