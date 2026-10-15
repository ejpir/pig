// Installed after host Web APIs and before the unchanged SDK bundle.
import process from './node/process.js';
import {Buffer} from './node/buffer.js';
Object.assign(globalThis,{process,Buffer,global:globalThis});
import '@formatjs/intl-getcanonicallocales/polyfill-force.js';
import '@formatjs/intl-locale/polyfill-force.js';
import '@formatjs/intl-segmenter/polyfill-force.js';
