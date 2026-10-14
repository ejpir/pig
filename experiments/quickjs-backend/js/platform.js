// Standards polyfills, not copies/patches of Pi or provider code.
import { URL, URLSearchParams } from 'whatwg-url';
import { AbortController, AbortSignal } from 'abort-controller/dist/abort-controller.mjs';
import { ReadableStream, WritableStream, TransformStream } from 'web-streams-polyfill';
import { Headers } from 'headers-polyfill';
import structuredClone from '@ungap/structured-clone';

Object.assign(globalThis, { URL, URLSearchParams, AbortController, AbortSignal,
  ReadableStream, WritableStream, TransformStream, Headers, structuredClone });
URL.canParse ??= (url, base) => { try { new URL(url, base); return true; } catch { return false; } };
globalThis.performance = { now: () => __now() };
globalThis.queueMicrotask = (fn) => Promise.resolve().then(fn);
// Quiet by default: provider debug logging can contain prompts/headers.
globalThis.console = { log() {}, info() {}, warn() {}, error() {}, debug() {}, assert() {} };

let nextId = 1;
const timers = new Map();
globalThis.setTimeout = (fn, ms = 0, ...args) => {
  if (typeof fn !== 'function') throw new TypeError('timer callback must be a function');
  const id = nextId++;
  timers.set(id, () => fn(...args));
  __timer(id, Math.max(0, Math.min(Number(ms) || 0, 2147483647)));
  return id;
};
globalThis.clearTimeout = (id) => { timers.delete(id); __clearTimer(id); };
globalThis.crypto = {
  getRandomValues(array) {
    if (!ArrayBuffer.isView(array) || array instanceof DataView || array instanceof Float32Array || array instanceof Float64Array || array.byteLength > 65536)
      throw new TypeError('expected an integer typed array of at most 65536 bytes');
    new Uint8Array(array.buffer, array.byteOffset, array.byteLength).set(__random(array.byteLength));
    return array;
  },
  randomUUID() {
    const bytes = crypto.getRandomValues(new Uint8Array(16));
    bytes[6] = (bytes[6] & 15) | 64; bytes[8] = (bytes[8] & 63) | 128;
    const hex = Array.from(bytes, b => b.toString(16).padStart(2, '0'));
    return [hex.slice(0,4), hex.slice(4,6), hex.slice(6,8), hex.slice(8,10), hex.slice(10)].map(x => x.join('')).join('-');
  },
};

// Narrow fetch/Response adapter: JSON POST and streaming response bodies only.
// URL policy, credentials, TLS, byte limits and actual I/O belong to Rust.
// This is not a complete browser fetch implementation (no uploads, clone, blobs, etc.).
class HostResponse {
  constructor(body, { status, headers, url }) {
    this.body = body; this.status = status; this.headers = new Headers(headers);
    this.url = url; this.ok = status >= 200 && status < 300;
    this.statusText = ''; this.bodyUsed = false;
  }
  async text() {
    if (this.bodyUsed) throw new TypeError('response body already used');
    this.bodyUsed = true;
    const decoder = new TextDecoder();
    let result = '';
    for await (const bytes of this.body) result += decoder.decode(bytes, { stream: true });
    return result + decoder.decode();
  }
  async json() { return JSON.parse(await this.text()); }
}
globalThis.Response = HostResponse;
// The SDK performs instanceof FormData even on JSON requests. Uploads fail closed.
globalThis.FormData = class FormData {
  constructor() { throw new Error('multipart uploads are outside this PoC'); }
};
const requests = new Map();
function abortError() { const error = new Error('Request aborted'); error.name = 'AbortError'; return error; }
globalThis.fetch = (url, init = {}) => new Promise((resolve, reject) => {
  if (init.signal?.aborted) { reject(abortError()); return; }
  if (typeof init.body !== 'string') { reject(new TypeError('only JSON string bodies supported')); return; }
  const id = nextId++;
  let controller;
  const clean = () => { requests.delete(id); init.signal?.removeEventListener('abort', abort); };
  const abort = () => {
    __cancelHttp(id); clean();
    const error = abortError();
    if (controller) controller.error(error);
    reject(error);
  };
  const body = new ReadableStream({
    start(value) { controller = value; },
    cancel() { __cancelHttp(id); clean(); },
  });
  requests.set(id, {
    headers(event) { resolve(new HostResponse(body, { ...event, url: String(url) })); },
    chunk(event) { controller.enqueue(new Uint8Array(event.bytes)); },
    done() { controller.close(); clean(); },
    error(event) { const error = new Error(event.message); controller.error(error); reject(error); clean(); },
  });
  init.signal?.addEventListener('abort', abort, { once: true });
  try {
    __http(id, JSON.stringify({ url: String(url), method: init.method || 'GET',
      headers: Object.fromEntries(new Headers(init.headers)), body: init.body }));
  } catch (error) {
    clean(); controller.error(error); reject(error);
  }
});
globalThis.hostDispatch = (json) => {
  const event = JSON.parse(json);
  if (event.kind === 'timer') {
    const fn = timers.get(event.id); timers.delete(event.id); fn?.();
  } else {
    requests.get(event.id)?.[event.kind]?.(event);
  }
};
