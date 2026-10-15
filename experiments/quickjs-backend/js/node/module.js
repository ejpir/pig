import fs from './fs.js';
const unavailable=()=>{throw new Error('Dynamic native/extension module loading is unavailable');};
export function createRequire(){const require=(name)=>{if(name==='fs'||name==='node:fs')return fs;return unavailable();};require.resolve=unavailable;return require;}
export const enableCompileCache=unavailable;
