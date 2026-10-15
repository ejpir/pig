import fs from './fs.js';
export const access=async(...args)=>fs.accessSync(...args),readFile=async(...args)=>fs.readFileSync(...args),writeFile=async(...args)=>fs.writeFileSync(...args),
 appendFile=async(...args)=>fs.appendFileSync(...args),realpath=async(...args)=>fs.realpathSync(...args),stat=async(...args)=>fs.statSync(...args),lstat=async(...args)=>fs.lstatSync(...args),
 mkdir=async(...args)=>fs.mkdirSync(...args),readdir=async(...args)=>fs.readdirSync(...args),rename=async(...args)=>fs.renameSync(...args),unlink=async(...args)=>fs.unlinkSync(...args),rm=async(...args)=>fs.rmSync(...args);
export async function open(){throw new Error('Descriptor stream adapter is unavailable');}
export default {access,readFile,writeFile,appendFile,realpath,stat,lstat,mkdir,readdir,rename,unlink,rm,open};
