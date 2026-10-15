import {Buffer} from './buffer.js';
// Framework filesystem authority is separate from model-controlled tool operations.
// No fallback to native paths, vault data, Node fs, or an unrestricted shell.
function call(op,path,options={}){
 if(typeof globalThis.__nodeFs!=='function'){const e=new Error('Filesystem adapter unavailable');e.code='ENOSYS';throw e;}
 const value=JSON.parse(globalThis.__nodeFs(JSON.stringify({op,path,...options})));
 if(value.error){const e=new Error(value.error.message||'Filesystem operation rejected');e.code=value.error.code||'EACCES';throw e;}
 return value.data;
}
export const constants={F_OK:0,R_OK:4,W_OK:2,X_OK:1};
export function readFileSync(path,encoding){const bytes=Buffer.from(call('read',String(path)),'base64');return typeof encoding==='string'?bytes.toString(encoding):encoding?.encoding?bytes.toString(encoding.encoding):bytes;}
export function writeFileSync(path,data,options){return call('write',String(path),{data:Buffer.from(data,typeof options==='string'?options:options?.encoding).toString('base64')});}
export function appendFileSync(path,data,options){return call('append',String(path),{data:Buffer.from(data,typeof options==='string'?options:options?.encoding).toString('base64')});}
export function accessSync(path,mode=0){call('access',String(path),{mode});}
export function existsSync(path){try{accessSync(path);return true;}catch{return false;}}
export function realpathSync(path){return call('realpath',String(path));}
export function statSync(path){const s=call('stat',String(path));return {...s,isDirectory:()=>s.kind==='directory',isFile:()=>s.kind==='file',isSymbolicLink:()=>false};}
export const lstatSync=statSync;
export function mkdirSync(path,options={}){return call('mkdir',String(path),{recursive:options.recursive===true});}
export function readdirSync(path,options={}){const list=call('readdir',String(path));return options.withFileTypes?list.map(s=>({...s,isDirectory:()=>s.kind==='directory',isFile:()=>s.kind==='file',isSymbolicLink:()=>false})):list.map(s=>s.name);}
export function renameSync(path,to){return call('rename',String(path),{to:String(to)});}
export function unlinkSync(path){return call('unlink',String(path));}
export function rmSync(path,options={}){return call('rm',String(path),{recursive:options.recursive===true,force:options.force===true});}
const unsupported=()=>{throw new Error('Filesystem operation has no admitted adapter');};
export const watch=unsupported,watchFile=unsupported,unwatchFile=unsupported,chmodSync=unsupported,copyFileSync=unsupported,
 globSync=unsupported,openSync=unsupported,closeSync=unsupported,readSync=unsupported,mkdtempSync=unsupported,createReadStream=unsupported,createWriteStream=unsupported;
export default {constants,readFileSync,writeFileSync,appendFileSync,accessSync,existsSync,realpathSync,statSync,lstatSync,mkdirSync,readdirSync,renameSync,unlinkSync,rmSync,
 watch,watchFile,unwatchFile,chmodSync,copyFileSync,globSync,openSync,closeSync,readSync,mkdtempSync,createReadStream,createWriteStream};
