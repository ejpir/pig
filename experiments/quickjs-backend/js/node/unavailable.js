// Optional native capabilities fail at use, never silently succeed.
function unavailable(){throw new Error('This Node capability has no admitted platform adapter');}
export const spawn=unavailable,spawnSync=unavailable,exec=unavailable,execSync=unavailable,execFile=unavailable,execFileSync=unavailable,
 createInterface=unavailable,createServer=unavailable,request=unavailable,get=unavailable,
 createWriteStream=unavailable,createReadStream=unavailable,pipeline=unavailable,
 Worker=unavailable,AsyncLocalStorage=unavailable,AsyncResource=unavailable,runInThisContext=unavailable,runInNewContext=unavailable,createContext=unavailable,compileFunction=unavailable,
 gunzipSync=unavailable,inflateSync=unavailable,brotliDecompressSync=unavailable,gzipSync=unavailable,
 serialize=unavailable,deserialize=unavailable,DatabaseSync=unavailable,createSecureContext=unavailable,
 createGunzip=unavailable,createInflate=unavailable,createBrotliDecompress=unavailable,createGzip=unavailable,crc32=unavailable,deflateRawSync=unavailable;
export const isatty=()=>false,parentPort=null,workerData=null,constants={},performance=globalThis.performance;
export default unavailable;
