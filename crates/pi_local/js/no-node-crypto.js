// Web Crypto is host-backed. Unselected Node-only OAuth branches fail closed.
export function randomBytes() { throw new Error('Node crypto OAuth branches are unavailable'); }
