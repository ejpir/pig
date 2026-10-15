export function fileURLToPath(value) {
 const u=new URL(String(value));
 if(u.protocol!=='file:' || (u.hostname && u.hostname!=='localhost'))throw new TypeError('Expected local file URL');
 if(/%2f/i.test(u.pathname))throw new TypeError('Encoded slash is not a file path');
 const path=decodeURIComponent(u.pathname);
 if(path.includes('\0'))throw new TypeError('Invalid file URL');
 return path;
}
export function pathToFileURL(path){if(!String(path).startsWith('/'))throw new TypeError('Expected absolute virtual path');return new URL('file://'+String(path).split('/').map(encodeURIComponent).join('/'));}
