export const setTimeout=(...args)=>globalThis.setTimeout(...args);
export const clearTimeout=(...args)=>globalThis.clearTimeout(...args);
export const setImmediate=(fn,...args)=>globalThis.setTimeout(fn,0,...args);
export const clearImmediate=(id)=>globalThis.clearTimeout(id);
