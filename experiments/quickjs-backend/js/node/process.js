import {EventEmitter} from './events.js';
const unsupported=()=>{throw new Error('Native process capability is unavailable');};
const process=new EventEmitter();
Object.assign(process,{platform:globalThis.__sdkPlatform?.platform??'android',arch:globalThis.__sdkPlatform?.arch??'unknown',features:{},getBuiltinModule:()=>undefined,versions:{},version:'',env:{PI_CODING_AGENT_DIR:'/pi/agent',PI_PACKAGE_DIR:'/pi-coding-agent'},argv:[],execPath:'/pi/runtime',pid:0,
 cwd:()=>'/workspace',chdir:unsupported,exit:unsupported,kill:unsupported,
 nextTick:(fn,...args)=>queueMicrotask(()=>fn(...args)),
 stdout:{isTTY:false,columns:80,rows:24,write:unsupported},stderr:{isTTY:false,write:unsupported},stdin:{isTTY:false},
 uptime:()=>performance.now()/1000,
 hrtime:(previous)=>{let ns=Math.floor(performance.now()*1e6);if(previous)ns-=previous[0]*1e9+previous[1];return [Math.floor(ns/1e9),ns%1e9];},
});
process.hrtime.bigint=()=>BigInt(Math.floor(performance.now()*1e6));
export default process;
export const {env,argv,stdout,stderr,stdin,cwd,nextTick}=process;
