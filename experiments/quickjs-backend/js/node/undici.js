// Node transport is not admitted in QuickJS. Direct public AgentSession construction
// does not configure it. Native fetch remains the only network transport.
const unavailable=()=>{throw new Error('Node HTTP dispatcher is unavailable on this platform');};
export class EnvHttpProxyAgent {constructor(){unavailable();}}
export class Client {constructor(){unavailable();}}
export class Pool {constructor(){unavailable();}}
export const setGlobalDispatcher=unavailable,install=unavailable;
