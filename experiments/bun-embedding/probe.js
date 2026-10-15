// Offline, trusted probe. No accounts, model-controlled code, HTTP or coding tools.
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';

const reportPath = process.argv[2];
if (!reportPath) throw new Error('A private report path is required');
const directory = path.dirname(reportPath);
const origin = JSON.parse(fs.readFileSync(path.join(directory, 'origin.json'), 'utf8'));
process.env.HOME = directory;
process.env.PI_CODING_AGENT_DIR = path.join(directory, 'agent');
process.env.PI_PACKAGE_DIR = path.join(directory, 'pi-coding-agent');
const nativeOrigin = origin.kind === 'jni-linked';
const report = {
  execution: nativeOrigin ? 'JNI-linked SDK probe' : 'desktop Bun CLI SDK smoke test (not Android/embedding evidence)',
  bunVersion: Bun.version, platform: process.platform, arch: process.arch,
  pid: process.pid, nativePid: origin.nativePid, javaPid: origin.javaPid,
  sameProcess: nativeOrigin ? process.pid === origin.nativePid && process.pid === origin.javaPid : null,
  syntheticOnly: true, tools: [],
};
let session;
try {
  const marker = path.join(directory, 'node-fs-probe.txt');
  fs.writeFileSync(marker, 'native Bun filesystem probe', {mode: 0o600});
  report.nodeFs = fs.readFileSync(marker, 'utf8') === 'native Bun filesystem probe';
  report.nodeCrypto = crypto.createHash('sha256').update('abc').digest('hex') ===
    'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad';
  const {Agent} = await import('@earendil-works/pi-agent-core');
  const {InMemoryCredentialStore, fauxProvider, fauxAssistantMessage} = await import('@earendil-works/pi-ai');
  const {AgentSession,ModelRuntime,SessionManager,SettingsManager,createExtensionRuntime,convertToLlm} =
    await import('@earendil-works/pi-coding-agent');
  const faux = fauxProvider({provider:'bun-embedding-fixture',models:[{
    id:'offline',contextWindow:200000,maxTokens:16384,input:['text'],reasoning:false,
  }],tokensPerSecond:100000});
  faux.setResponses([
    fauxAssistantMessage('Old facts acknowledged.'),
    fauxAssistantMessage('Recent facts acknowledged.'),
    fauxAssistantMessage('## Goal\nPreserve old and recent facts.\n## Progress\nOffline linked-runtime probe only.'),
  ]);
  const runtime = await ModelRuntime.create({credentials:new InMemoryCredentialStore(),
    modelsPath:null,refreshOnCreate:false,allowModelNetwork:false});
  runtime.registerNativeProvider(faux.provider);
  const model = runtime.getModel(faux.provider.id,'offline');
  const settings = SettingsManager.inMemory({transport:'sse',cacheWarming:'off',retry:{enabled:false},
    compaction:{enabled:true,reserveTokens:16384,keepRecentTokens:20000}});
  const manager = SessionManager.inMemory(directory);
  const extensions = createExtensionRuntime();
  const resources = {
    getExtensions:()=>({extensions:[],errors:[],runtime:extensions}),
    getSkills:()=>({skills:[],diagnostics:[]}),getPrompts:()=>({prompts:[],diagnostics:[]}),
    getThemes:()=>({themes:[],diagnostics:[]}),getAgentsFiles:()=>({agentsFiles:[]}),
    getSystemPrompt:()=>undefined,getSystemPromptSource:()=>undefined,
    getAppendSystemPrompt:()=>[],getAppendSystemPromptSources:()=>[],
    reload:async()=>{},extendResources:()=>{throw Error('Resources are not admitted');},
  };
  const runnerRef = {};
  const agent = new Agent({initialState:{model,thinkingLevel:'off',tools:[],messages:[]},convertToLlm,
    streamFn:(m,c,o)=>runtime.streamSimple(m,c,o),sessionId:manager.getSessionId(),
    transformContext:async messages=>runnerRef.current?runnerRef.current.emitContext(messages):messages});
  session = new AgentSession({agent,sessionManager:manager,settingsManager:settings,modelRuntime:runtime,
    resourceLoader:resources,cwd:directory,baseToolsOverride:{},initialActiveToolNames:[],allowedToolNames:[],extensionRunnerRef:runnerRef});
  await session.bindExtensions({mode:'rpc',onError:()=>{throw Error('Extension failure');}});
  report.sdkConstructed = session.constructor.name === 'AgentSession';
  await session.prompt('old facts '.repeat(10000),{expandPromptTemplates:false});
  await session.prompt('recent facts '.repeat(10000),{expandPromptTemplates:false});
  const id = manager.getSessionId(),before = manager.getEntries();
  const compact = await session.compact('Preserve the facts.');
  report.compaction = compact.summary.includes('Preserve old and recent facts.');
  report.unknownPostCompactionUsage = session.getContextUsage().tokens === null;
  const all = [manager.getHeader(),...manager.getEntries()];
  const restored = SessionManager.inMemory(directory,{id},JSON.parse(JSON.stringify(all)));
  restored.branch(manager.getLeafId());
  report.checkpointRestored = restored.getSessionId() === id &&
    JSON.stringify(restored.buildSessionContext()) === JSON.stringify(manager.buildSessionContext());
  report.oldEntriesPreserved = JSON.stringify(restored.getEntries().slice(0,before.length)) === JSON.stringify(before);
  report.syntheticCalls = faux.state.callCount;
  report.tools = session.getActiveToolNames();
  report.passed = (origin.kind === 'host-cli-smoke' || report.sameProcess) && report.nodeFs && report.nodeCrypto && report.sdkConstructed &&
    report.compaction && report.checkpointRestored && report.oldEntriesPreserved && report.unknownPostCompactionUsage && report.tools.length === 0;
} catch(error) {
  report.passed = false;
  // This package has no credentials or network permission; diagnostics are offline only.
  report.error = `${error.name}: ${error.message}`;
} finally {
  session?.dispose();
  fs.writeFileSync(reportPath,JSON.stringify(report,null,2)+'\n',{mode:0o600});
}
