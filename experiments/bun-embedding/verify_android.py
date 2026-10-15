#!/usr/bin/env python3
"""Validate only the separate, credential-free linked-Bun test APK on an emulator."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import time

PACKAGE='dev.pi.bunembedding'
ACTIVITY=PACKAGE+'/'+PACKAGE+'.ProbeActivity'

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--adb',default='adb')
    p.add_argument('--serial',default='emulator-5554')
    p.add_argument('--apk',type=Path,required=True)
    p.add_argument('--output',type=Path,default=Path('artifacts/bun-embedding-emulator'))
    p.add_argument('--durable',action='store_true',help='Require bundled SDK, real tools and a second-process SQLite reopen')
    args=p.parse_args()
    if not args.serial.startswith('emulator-'):raise SystemExit('This proof harness is emulator-only')
    args.output.mkdir(parents=True,exist_ok=True)
    def adb(*parts,check=True):
        return subprocess.run([args.adb,'-s',args.serial,*parts],check=check,capture_output=True,timeout=45)
    install=adb('install','--no-incremental','-r',str(args.apk.resolve()))
    (args.output/'install.txt').write_bytes(install.stdout+install.stderr)
    reports=[]
    for phase in (['create','recover'] if args.durable else ['create']):
        adb('shell','am','force-stop',PACKAGE)
        # Only this synthetic package's report is removed; its SQLite state is retained.
        adb('shell','run-as',PACKAGE,'rm','-f','files/report.json')
        launch=adb('shell','am','start','-W','-n',ACTIVITY,'--es','durable_phase',phase)
        (args.output/f'launch-{phase}.txt').write_bytes(launch.stdout+launch.stderr)
        started=time.monotonic();deadline=started+75
        report=None
        while time.monotonic()<deadline:
            response=adb('shell','run-as',PACKAGE,'cat','files/report.json',check=False)
            if response.returncode==0:
                try:report=json.loads(response.stdout);break
                except ValueError:pass
            if time.monotonic()-started>3 and adb('shell','pidof',PACKAGE,check=False).returncode!=0:
                raise SystemExit(f'Test process exited during {phase}; no fresh JNI report')
            time.sleep(.2)
        if report is None:raise SystemExit(f'No fresh JNI report for {phase}; failure is not a pass')
        (args.output/f'runtime-report-{phase}.json').write_text(json.dumps(report,indent=2)+'\n')
        assert report['passed'] and report['returnedToJava'] and report['initStatus']==0,report
        assert report['javaPid']==report['nativePid']
        for i,result in enumerate(report['results']):
            assert result['bun'] in ('1.4.3','1.4.3-debug') and result['buffer']=='6f6b'
            assert result['pid']==report['nativePid'] and result['count']==i+1
        if args.durable:
            assert report['durableSdkBundled'] and report['durable']['passed'],report
            sdk=report['durable']
            assert sdk['phase']==phase and sdk['pid']==report['nativePid']
            assert sdk['tools']==['write','read','edit','bash','read'] and sdk['fileContent']=='after\n'
            assert sdk['shellPath']=='/system/bin/sh' and sdk['commandOutput']==['shell-ok:after\n']
            assert sdk['syntheticModelCalls']==(6 if phase=='create' else 0)
            assert sdk['sqliteReopened'] and sdk['completedReceiptRestored'] and sdk['completedRequestDeduplicated']
        reports.append(report)
    if args.durable:
        assert reports[0]['nativePid']!=reports[1]['nativePid'],'Reopen must use a fresh Android process/VM'
        for field in ('conversationId','submissionId','databasePath'):
            assert reports[0]['durable'][field]==reports[1]['durable'][field],f'Persisted {field} must survive'
    (args.output/'runtime-report.json').write_text(json.dumps(report,indent=2)+'\n')
    time.sleep(2)
    pid=adb('shell','pidof',PACKAGE).stdout.decode().strip()
    assert str(report['nativePid']) in pid.split(),'The host must remain alive after Java receives both results'
    # No screenshot of other apps/accounts: prove the PoC still owns the foreground.
    resumed=adb('shell','dumpsys','activity','activities').stdout.decode(errors='replace')
    foreground=[line for line in resumed.splitlines() if 'topResumedActivity=' in line or 'mResumedActivity:' in line]
    assert any(PACKAGE in line for line in foreground),'PoC is not the resumed Activity'
    (args.output/'screen.png').write_bytes(adb('exec-out','screencap','-p').stdout)
    evidence={'passed':True,'scope':('Android x86_64 emulator, embedded Durable SDK/tool execution + fresh-process SQLite reopen' if args.durable else 'Android x86_64 emulator, trusted synchronous JNI embedding only'),
      'apkSha256':hashlib.sha256(args.apk.read_bytes()).hexdigest(),'pid':int(pid.split()[0]),
      'noRealProviders':True,'existingAppAndAccountsUntouched':True,
      'checks':['source-built shared library','System.loadLibrary + JNI (no Bun executable)',
        'real Bun.version and Buffer','two calls preserve one VM','same Java/native/Bun PID',
        'returns to Java','process/Activity survives','no INTERNET permission in test manifest']}
    if args.durable:
        evidence['checks']+=['unchanged pi-durable bundled in APK','real SDK tools','SQLite session identity survives process restart']
        evidence['processPids']=[entry['nativePid'] for entry in reports]
    (args.output/'report.json').write_text(json.dumps(evidence,indent=2)+'\n')
    print('PASS: '+('embedded Durable SDK/tools and fresh-process SQLite reopen' if args.durable else 'real Bun runs through JNI twice and returns inside the same Android process'))

if __name__=='__main__':main()
