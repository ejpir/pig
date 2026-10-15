// Platform adapter; POSIX path algorithms are the unchanged browserify implementation.
import path from 'path-browserify';
export const {resolve, normalize, isAbsolute, join, relative, dirname, basename, extname, parse, format, sep, delimiter, posix}=path;
export const toNamespacedPath=path=>path;
// Windows filesystem semantics are not a supported Android capability.
export const win32=undefined;
export default path;
