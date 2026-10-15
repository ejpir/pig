// Standards polyfills, not copies/patches of Pi or provider code.
import { URL, URLSearchParams } from 'whatwg-url';
import { AbortController as BaseAbortController, AbortSignal } from 'abort-controller/dist/abort-controller.mjs';
import { ReadableStream, WritableStream, TransformStream } from 'web-streams-polyfill';
import { Headers } from 'headers-polyfill';
import structuredClone from '@ungap/structured-clone';

const abortReasons = new WeakMap();
class AbortController extends BaseAbortController {
  abort(reason = abortError()) {
    if (!this.signal.aborted) abortReasons.set(this.signal, reason);
    super.abort();
  }
}
Object.defineProperty(AbortSignal.prototype, 'reason', { get() { return abortReasons.get(this); } });
AbortSignal.prototype.throwIfAborted = function () { if (this.aborted) throw this.reason ?? abortError(); };
AbortSignal.abort = (reason) => { const c = new AbortController(); c.abort(reason); return c.signal; };
AbortSignal.timeout = (ms) => {
  const c = new AbortController();
  setTimeout(() => { const e = new Error('Timed out'); e.name = 'TimeoutError'; c.abort(e); }, ms);
  return c.signal;
};
AbortSignal.any = (signals) => {
  const c = new AbortController(); const subscriptions = [];
  const clean = () => { for (const [signal, fn] of subscriptions) signal.removeEventListener('abort', fn); subscriptions.length = 0; };
  for (const signal of signals) {
    if (signal.aborted) { c.abort(signal.reason); clean(); break; }
    const fn = () => { c.abort(signal.reason); clean(); };
    subscriptions.push([signal, fn]); signal.addEventListener('abort', fn, { once: true });
  }
  return c.signal;
};
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
  subtle: {
    async digest(algorithm, data) {
      if (String(typeof algorithm === 'string' ? algorithm : algorithm.name).toUpperCase() !== 'SHA-256')
        throw new Error('Only SHA-256 is supported');
      const bytes = ArrayBuffer.isView(data) ? new Uint8Array(data.buffer, data.byteOffset, data.byteLength) : new Uint8Array(data);
      return new Uint8Array(__sha256(Array.from(bytes))).buffer;
    },
  },
  randomUUID() {
    const bytes = crypto.getRandomValues(new Uint8Array(16));
    bytes[6] = (bytes[6] & 15) | 64; bytes[8] = (bytes[8] & 63) | 128;
    const hex = Array.from(bytes, b => b.toString(16).padStart(2, '0'));
    return [hex.slice(0,4), hex.slice(4,6), hex.slice(6,8), hex.slice(8,10), hex.slice(10)].map(x => x.join('')).join('-');
  },
};

globalThis.btoa = (text) => {
  const bytes = Array.from(String(text), char => char.charCodeAt(0));
  if (bytes.some(byte => byte > 255)) throw new TypeError('btoa expects Latin-1');
  return __btoa(bytes);
};
globalThis.atob = text => __atob(String(text).replace(/[\t\n\f\r ]/g, '')).map(byte => String.fromCharCode(byte)).join('');

// Narrow fetch/Response adapter: JSON/form requests and streaming response bodies.
// URL policy, credentials, TLS, byte limits and actual I/O belong to Rust.
// This is not a complete browser fetch implementation (no uploads, clone, blobs, etc.).
class HostResponse {
  constructor(body, { status = 200, headers, url = '', statusText = '' } = {}) {
    // Upstream Codex constructs Response(errorText, init) when handling non-2xx.
    if (typeof body === 'string') {
      const bytes = new TextEncoder().encode(body);
      body = new ReadableStream({ start(controller) { controller.enqueue(bytes); controller.close(); } });
    }
    this.body = body; this.status = status; this.headers = new Headers(headers);
    this.url = url; this.ok = status >= 200 && status < 300;
    this.statusText = statusText; this.bodyUsed = false;
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
  const headers = new Headers(init.headers);
  let requestBody = init.body;
  if (requestBody instanceof URLSearchParams) {
    requestBody = requestBody.toString();
    if (!headers.has('content-type')) headers.set('content-type', 'application/x-www-form-urlencoded;charset=UTF-8');
  }
  if (requestBody != null && typeof requestBody !== 'string') { reject(new TypeError('only text/form bodies supported')); return; }
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
      headers: Object.fromEntries(headers), body: requestBody ?? null }));
  } catch (error) {
    clean(); controller.error(error); reject(error);
  }
});
globalThis.hostDispatch = (json) => {
  const event = JSON.parse(json);
  if (event.kind === 'timer') {
    const fn = timers.get(event.id); timers.delete(event.id); fn?.();
  } else if (event.kind === 'input') {
    globalThis.hostInput?.(event);
  } else {
    requests.get(event.id)?.[event.kind]?.(event);
  }
};
