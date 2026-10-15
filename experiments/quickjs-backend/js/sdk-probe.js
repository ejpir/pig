// Offline public-construction/checkpoint compatibility evidence.
// All Pi behavior is imported unchanged through public exports.
import {Agent} from '@earendil-works/pi-agent-core';
import {InMemoryCredentialStore} from '@earendil-works/pi-ai';
import {AgentSession,ModelRuntime,SessionManager,SettingsManager,createExtensionRuntime,convertToLlm} from '@earendil-works/pi-coding-agent';

async function construct() {
 const cwd='/workspace';
 const settingsManager=SettingsManager.inMemory({transport:'sse',cacheWarming:'off',images:{autoResize:false}});
 const sessionManager=SessionManager.inMemory(cwd);
 const credentials=new InMemoryCredentialStore();
 const modelRuntime=await ModelRuntime.create({credentials,modelsPath:null,refreshOnCreate:false,allowModelNetwork:false});
 const model=modelRuntime.getModels('anthropic')[0];
 if(!model)throw new Error('Missing upstream catalog');
 const resourceLoader={
  getExtensions:()=>({extensions:[],errors:[],runtime:createExtensionRuntime()}),
  getSkills:()=>({skills:[],diagnostics:[]}),getPrompts:()=>({prompts:[],diagnostics:[]}),getThemes:()=>({themes:[],diagnostics:[]}),
  getAgentsFiles:()=>({agentsFiles:[]}),getSystemPrompt:()=>undefined,getSystemPromptSource:()=>undefined,
  getAppendSystemPrompt:()=>[],getAppendSystemPromptSources:()=>[],
  reload:async()=>{},extendResources:()=>{throw new Error('Resources are not admitted in the construction probe');},
 };
 const runnerRef={};
 const agent=new Agent({initialState:{model,thinkingLevel:'off',tools:[],messages:sessionManager.buildSessionContext().messages},convertToLlm,
  streamFn:()=>{throw new Error('Inference is not admitted in the construction probe');},
  transformContext:async messages=>runnerRef.current?runnerRef.current.emitContext(messages):messages,
  sessionId:sessionManager.getSessionId(),steeringMode:settingsManager.getSteeringMode(),followUpMode:settingsManager.getFollowUpMode(),
  transport:settingsManager.getTransport(),thinkingBudgets:settingsManager.getThinkingBudgets(),
 });
 const session=new AgentSession({agent,sessionManager,settingsManager,modelRuntime,resourceLoader,cwd,
  baseToolsOverride:{},initialActiveToolNames:[],allowedToolNames:[],extensionRunnerRef:runnerRef});
 await session.bindExtensions({mode:'rpc',onError:()=>{throw new Error('Extension binding rejected');}});
 try {
  const id=sessionManager.getSessionId();
  sessionManager.appendMessage({role:'user',content:'Canonical probe message',timestamp:0});
  const context=sessionManager.buildSessionContext();
  if(context.messages.length!==1||context.messages[0].content!=='Canonical probe message')throw new Error('Upstream session context');
  const root=sessionManager.getLeafId();
  const other=sessionManager.appendMessage({role:'user',content:'Other branch',timestamp:1});
  sessionManager.branch(root);
  const compact=sessionManager.appendCompaction('Summary sentinel',undefined,123);
  sessionManager.appendMessage({role:'user',content:'Post-summary',timestamp:2});
  const all=[sessionManager.getHeader(),...sessionManager.getEntries()];
  const roundTrips=[];
  for(const leaf of [other,compact,null]) {
   const restored=SessionManager.inMemory(cwd,{id:'ignored-option-id'},JSON.parse(JSON.stringify(all)));
   if(leaf===null)restored.resetLeaf();else restored.branch(leaf);
   if(restored.getSessionId()!==id||restored.getEntries().length!==all.length-1)throw Error('Graph restoration');
   const checkpoint={sessionId:id,cwd,fileEntries:[restored.getHeader(),...restored.getEntries()],leafId:restored.getLeafId(),messages:restored.buildSessionContext().messages};
   roundTrips.push(checkpoint);
  }
  return {roundTrips,className:session.constructor.name,sessionId:id,stableId:id===sessionManager.getSessionId(),tools:session.getActiveToolNames(),
   systemPromptLength:session.systemPrompt.length,entries:sessionManager.getEntries().length,messages:context.messages.length};
 } finally {session.dispose();}
}
globalThis.runSdkProbe=async()=>{try{__finish(JSON.stringify(await construct()));}catch(error){__finish(JSON.stringify({error:error.message,stack:error.stack}));}};
