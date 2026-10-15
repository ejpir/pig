#!/usr/bin/env python3
"""Relink Bun's real Android PIC object graph plus our JNI bridge as a .so.

No executable/ELF patching. Reuses the actual upstream-generated link inputs,
changes only the final link mode/export controls and adds the caller object.
"""
import argparse
from pathlib import Path
import shlex
import subprocess
import json

ROOT=Path(__file__).resolve().parent

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--source',type=Path,required=True)
    p.add_argument('--build',type=Path,required=True)
    p.add_argument('--ninja',type=Path,required=True)
    p.add_argument('--jni-object',type=Path,required=True)
    p.add_argument('--out',type=Path,required=True)
    args=p.parse_args()
    source=args.source.resolve();build=args.build.resolve();out=args.out.resolve()
    commands=subprocess.check_output([str(args.ninja),'-C',str(build),'-t','commands','bun-debug'],text=True)
    candidates=[line for line in commands.splitlines() if 'clang++ @bun-debug.rsp ' in line]
    if len(candidates)!=1:raise SystemExit(f'Expected one genuine Bun link command, found {len(candidates)}')
    command=shlex.split(candidates[0]);changed=False
    for i,arg in enumerate(command):
        if arg=='-pie':command[i]='-shared';changed=True
    if not changed:raise SystemExit('Upstream Android link mode changed; audit before relinking')
    # Save exact command BEFORE modifications for provenance.
    out.parent.mkdir(parents=True,exist_ok=True)
    (out.parent/'upstream-link-command.json').write_text(json.dumps(shlex.split(candidates[0]),indent=2)+'\n')
    linker=(source/'src/linker.lds').read_text()
    if 'global:' not in linker:raise SystemExit('Unknown upstream version-script layout')
    linker=linker.replace('global:','global:\n    JNI_OnLoad;\n    pi_bun_*;\n    Java_dev_pi_bunembedding_ProbeActivity_*;',1)
    version_script=out.parent/'bun-jni.lds';version_script.write_text(linker)
    exports=out.parent/'bun-jni.exports'
    exports.write_text((build/'exports.list').read_text().replace('{','{\n  JNI_OnLoad;\n  pi_bun_*;\n  Java_dev_pi_bunembedding_ProbeActivity_*;',1))
    for i,arg in enumerate(command):
        if arg.startswith('-Wl,--version-script='):command[i]='-Wl,--version-script='+str(version_script)
        if arg.startswith('-Wl,--export-dynamic-symbol-list='):command[i]='-Wl,--export-dynamic-symbol-list='+str(exports)
    # -u,main keeps the original full-runtime root, but we NEVER call main.
    # This rules out misleading tiny libraries with most of Bun GC'd away.
    command.extend([str(args.jni_object.resolve()),'-Wl,-u,main','-Wl,-z,defs','-Wl,-z,relro','-Wl,-z,now',
                    '-Wl,-soname,libbunembeddingpoc.so'])
    output_index=command.index('-o')+1;command[output_index]=str(out)
    rsp=build/'bun-debug.rsp'
    if not rsp.exists():
        # Ninja normally removes response files. Only emit the same upstream
        # edge's inputs; never synthesize a separate runtime/object inventory.
        raise SystemExit('Missing original Bun response file; build with -d keeprsp')
    (out.parent/'jni-link-command.json').write_text(json.dumps(command,indent=2)+'\n')
    print('Relinking source-built Bun PIC objects as',out,flush=True)
    subprocess.run(command,cwd=build,check=True)
    (out.parent/'LICENSE.bun.md').write_bytes((source/'LICENSE.md').read_bytes())
    subprocess.run(['readelf','-h',str(out)],check=True)

if __name__=='__main__':main()
