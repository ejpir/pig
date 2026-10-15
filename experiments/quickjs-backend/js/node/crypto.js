import {Buffer} from './buffer.js';
export const randomUUID=()=>globalThis.crypto.randomUUID();
export function randomBytes(size){if(!Number.isInteger(size)||size<0||size>65536)throw new RangeError('Random byte limit');return Buffer.from(globalThis.crypto.getRandomValues(new Uint8Array(size)));}
export function createHash(algorithm){if(algorithm!=='sha256')throw new Error('Unsupported hash algorithm');let parts=[],size=0,finished=false;return {update(value){if(finished)throw new Error('Digest already finished');let bytes=Buffer.from(value);size+=bytes.length;if(size>1024*1024)throw new Error('Digest input limit');parts.push(bytes);return this;},digest(encoding){if(finished)throw new Error('Digest already finished');finished=true;const result=Buffer.from(__sha256(Array.from(Buffer.concat(parts))));return encoding?result.toString(encoding):result;}};}
export const getHashes=()=>['sha256'];
export const webcrypto=globalThis.crypto;
