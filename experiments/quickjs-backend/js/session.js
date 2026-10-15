// Platform/protocol adapter only. Canonical entries, context projection, compaction,
// overflow recovery and summarization are the unchanged public upstream SDK.
import {Agent} from '@earendil-works/pi-agent-core';
import {InMemoryCredentialStore,isContextOverflow,AssistantMessageEventStream} from '@earendil-works/pi-ai';
import {clampThinkingLevel} from '@earendil-works/pi-ai/models';
import {anthropicProvider} from '@earendil-works/pi-ai/providers/anthropic';
import {openaiCodexProvider} from '@earendil-works/pi-ai/providers/openai-codex';
import {AgentSession,ModelRuntime,SessionManager,SettingsManager,createExtensionRuntime,convertToLlm,
 estimateTokens,shouldCompact} from '@earendil-works/pi-coding-agent';

function resources() {
 const runtime=createExtensionRuntime();
 return {getExtensions:()=>({extensions:[],errors:[],runtime}),
  getSkills:()=>({skills:[],diagnostics:[]}),getPrompts:()=>({prompts:[],diagnostics:[]}),getThemes:()=>({themes:[],diagnostics:[]}),
  getAgentsFiles:()=>({agentsFiles:[]}),getSystemPrompt:()=>undefined,getSystemPromptSource:()=>undefined,
  getAppendSystemPrompt:()=>[],getAppendSystemPromptSources:()=>[],reload:async()=>{},
  extendResources:()=>{throw new Error('Resource loading is not admitted');}};
}
function publicMessage(m) {
 const {role,content,api,provider,model,usage,stopReason,timestamp,thinkingLevel}=m;
 return {role,content,api,provider,model,usage,stopReason,timestamp,thinkingLevel,
  ...(['error','aborted'].includes(stopReason)?{errorMessage:'Reply did not complete.'}:{})};
}
globalThis.startSessionOperation=async json=>{
 let session,manager,settings,model,lastAssistant,compactionActive=false,compactionFailed=false,bridgeFailed=false,stopped=false,compactionResult,ok=false,safeError='failed';
 const input=JSON.parse(json),encoder=new TextEncoder();
 let eventBytes=0,events=0;
 const emit=record=>{
  if(bridgeFailed)return;
  try {
   const encoded=JSON.stringify({kind:'record',record});eventBytes+=encoder.encode(encoded).length;
   if(++events>20000||eventBytes>4*1024*1024)throw Error('Event bound');
   __emit(encoded);
  } catch {
   bridgeFailed=true;
   // Session listeners must not throw before SDK message persistence.
   Promise.resolve().then(()=>session?.abort()).catch(()=>{});
  }
 };
 globalThis.hostInput=event=>{if(event.action==='abort'){stopped=true;session?.abort().catch(()=>{});}};
 const checkStop=()=>{stopped=stopped||__operationCancelled();if(stopped)throw Error('Stopped');};
 const noCompaction=error=>['Nothing to compact (session too small)','Already compacted'].includes(error.message);
 let baseline=0;
 try {
  const prepared=JSON.parse(__resolvedAuth());
  if(prepared.source!=='OAuth'||!prepared.auth)throw Error('Missing prepared auth');
  const stock=input.provider==='anthropic'?anthropicProvider():openaiCodexProvider();
  const boundedStream=(invoke,m,c,options)=>{
   checkStop();
   const controller=new AbortController(),parent=options?.signal,stream=new AssistantMessageEventStream();
   const abort=()=>controller.abort();parent?.addEventListener('abort',abort,{once:true});if(parent?.aborted)abort();
   const failed=()=>{const message={role:'assistant',content:[],api:m.api,provider:m.provider,model:m.id,timestamp:Date.now(),
    usage:{input:0,output:0,cacheRead:0,cacheWrite:0,totalTokens:0,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}},stopReason:'error',errorMessage:'Reply did not complete.'};
    stream.push({type:'error',reason:'error',error:message});stream.end(message);};
   (async()=>{try{
    for await(const event of invoke(m,c,{...options,signal:controller.signal})){
     const frame=event.partial??event.message??event.error;
     if(frame&&(encoder.encode(JSON.stringify(frame)).length>512*1024||(frame.content??[]).reduce((n,b)=>n+encoder.encode(b.text??b.thinking??'').length,0)>256*1024)){controller.abort();failed();return;}
     // Server diagnostics are not canonical model/user content. Never persist them.
     stream.push(event.type==='error'?{...event,error:{...event.error,errorMessage:isContextOverflow(event.error,m.contextWindow)?'context length exceeded':'Reply did not complete.'}}:event);
    }
    stream.end();
   }catch{controller.abort();failed();}finally{parent?.removeEventListener('abort',abort);}})();return stream;
  };
  const admittedOptions=options=>({...options,maxTokens:options?.maxTokens??4096,transport:'sse',maxRetries:0,timeoutMs:180000,toolChoice:'none'});
  const preparedProvider={id:stock.id,name:stock.name,baseUrl:stock.baseUrl,headers:stock.headers,getModels:()=>stock.getModels(),
   auth:{apiKey:{name:'Native prepared operation',check:async({signal})=>{signal?.throwIfAborted();return {type:'api_key',source:prepared.source};},
    resolve:async({signal})=>{signal?.throwIfAborted();return {auth:prepared.auth,env:prepared.env,source:prepared.source};}}},
   stream:(m,c,o)=>boundedStream(stock.stream,m,c,admittedOptions(o)),streamSimple:(m,c,o)=>boundedStream(stock.streamSimple,m,c,admittedOptions(o))};
  const runtime=await ModelRuntime.create({credentials:new InMemoryCredentialStore(),modelsPath:null,refreshOnCreate:false,allowModelNetwork:false});
  checkStop();runtime.registerNativeProvider(preparedProvider);
  model=runtime.getModel(input.provider,input.model);
  if(!model)throw Error('Unknown model');
  settings=SettingsManager.inMemory({transport:'sse',cacheWarming:'off',images:{autoResize:false},retry:{enabled:false},
   compaction:{enabled:true,reserveTokens:16384,keepRecentTokens:20000}});
  manager=SessionManager.inMemory('/workspace',{id:input.sessionId},input.checkpoint?.fileEntries);
  if(input.checkpoint){if(input.checkpoint.leafId===null)manager.resetLeaf();else manager.branch(input.checkpoint.leafId);}
  else for(const message of input.messages.slice(0,input.operation==='compact'?undefined:-1))manager.appendMessage(message);
  const previous=manager.buildSessionContext();
  if(previous.model?.provider!==input.provider||previous.model?.modelId!==input.model)manager.appendModelChange(input.provider,input.model);
  if(previous.thinkingLevel!==input.thinkingLevel)manager.appendThinkingLevelChange(input.thinkingLevel);
  baseline=manager.getEntries().length;
  const runnerRef={};
  const agent=new Agent({initialState:{model,thinkingLevel:clampThinkingLevel(model,input.thinkingLevel),tools:[],messages:manager.buildSessionContext().messages},
   convertToLlm,transformContext:async messages=>runnerRef.current?runnerRef.current.emitContext(messages):messages,
   streamFn:(m,c,o)=>runtime.streamSimple(m,c,admittedOptions(o)),sessionId:manager.getSessionId(),transport:'sse',
   steeringMode:settings.getSteeringMode(),followUpMode:settings.getFollowUpMode(),thinkingBudgets:settings.getThinkingBudgets()});
  session=new AgentSession({agent,sessionManager:manager,settingsManager:settings,modelRuntime:runtime,resourceLoader:resources(),cwd:'/workspace',
   baseToolsOverride:{},initialActiveToolNames:[],allowedToolNames:[],extensionRunnerRef:runnerRef});
  await session.bindExtensions({mode:'rpc',onError:()=>{bridgeFailed=true;}});
  let outputBytes=0;const blocks=new Map();
  session.subscribe(event=>{
   // Only normalized public records cross the presentation bridge, never raw errors/auth.
   try {
    if(event.type==='message_start'&&event.message.role==='assistant'){outputBytes=0;blocks.clear();emit({type:event.type,message:publicMessage(event.message)});}
    else if(event.type==='message_end'&&event.message.role==='assistant'){
     const m=event.message;lastAssistant=m;
     if(encoder.encode(JSON.stringify(m)).length>512*1024||(m.content??[]).reduce((n,b)=>n+encoder.encode(b.text??b.thinking??'').length,0)>256*1024)throw Error('Output bound');
     emit({type:event.type,message:publicMessage(m)});
    } else if(event.type==='message_update'){
     const e=event.assistantMessageEvent;
     if(/^(text|thinking)_(start|delta|end)$/.test(e.type)){
      const old=blocks.get(e.contentIndex)??0,size=e.type.endsWith('_delta')?old+encoder.encode(e.delta??'').length:encoder.encode(e.content??'').length;
      outputBytes+=size-old;blocks.set(e.contentIndex,size);if(outputBytes>256*1024)throw Error('Output bound');
      const {type,contentIndex,delta,content}=e;emit({type:event.type,assistantMessageEvent:{type,contentIndex,delta,content}});
     }
    } else if(event.type==='compaction_start'){compactionActive=true;emit({type:event.type,reason:event.reason});}
    else if(event.type==='compaction_end'){
     if(event.result)compactionResult=event.result;
     if(event.errorMessage&&compactionActive&&!noCompaction({message:event.errorMessage}))compactionFailed=true;
     compactionActive=false;
     emit({type:event.type,reason:event.reason,aborted:event.aborted,
      ...(event.result?{result:{summary:event.result.summary,tokensBefore:event.result.tokensBefore,estimatedTokensAfter:event.result.estimatedTokensAfter}}:{}),
      ...(event.errorMessage?{errorMessage:'Context summarization did not complete.'}:{})});
    }
   } catch {bridgeFailed=true;Promise.resolve().then(()=>session.abort()).catch(()=>{});}
  });
  checkStop();
  if(input.operation==='compact'){
   compactionResult=await session.compact(input.customInstructions);ok=true;
  } else {
   const user=input.messages.at(-1),content=user.content;
   const text=typeof content==='string'?content:content.filter(b=>b.type==='text').map(b=>b.text).join('\n');
   const images=typeof content==='string'?[]:content.filter(b=>b.type==='image');
   // Stock physical-model preflight omits the pending prompt. Application admission
   // uses upstream estimates and public compact(), without copying cut/summarize logic.
   const usage=session.getContextUsage();
   const tokens=usage?.tokens??manager.buildSessionContext().messages.reduce((n,m)=>n+estimateTokens(m),0);
   const policy=settings.getCompactionSettings(model);
   if(shouldCompact(tokens+estimateTokens(user),model.contextWindow,policy)&&manager.buildSessionContext().messages.length){
    try{await session.compact();}catch(error){if(!noCompaction(error))throw error;compactionFailed=false;} // Public API reason stays manual.
    checkStop();
   }
   const after=session.getContextUsage()?.tokens??manager.buildSessionContext().messages.reduce((n,m)=>n+estimateTokens(m),0);
   if(shouldCompact(after+estimateTokens(user),model.contextWindow,policy)){safeError='context_full';throw Error('Input exceeds context');}
   checkStop();await session.prompt(text,{images,expandPromptTemplates:false});
   const last=lastAssistant;
   ok=last?.role==='assistant'&&['stop','length'].includes(last.stopReason)&&!bridgeFailed&&!stopped;
   if(!ok&&last&&isContextOverflow(last,model.contextWindow))safeError=compactionFailed?'compaction_failed':'context_full';
  }
 } catch(error) {
  // Only these fixed classifications reach native/UI. Never raw server diagnostics.
  if(noCompaction(error))safeError='nothing_to_compact';
  else if(compactionFailed)safeError='compaction_failed';
 } finally {
  try {
  if(session){await session.waitForIdle().catch(()=>{});}
  if(manager&&!ok&&input.operation!=='compact'){
   // Preserve the existing Local failure policy using public context edits, not a
   // second message-history implementation. SDK overflow recovery edits remain intact.
   for(const entry of manager.getEntries().slice(baseline))if(entry.type==='message'&&['user','assistant'].includes(entry.message.role))manager.appendContextEdit(entry.id,null);
  }
  const checkpoint=manager?{sessionId:manager.getSessionId(),cwd:manager.getCwd(),fileEntries:[manager.getHeader(),...manager.getEntries()],leafId:manager.getLeafId()}:undefined;
  if(manager)__sessionContext(JSON.stringify(manager.buildSessionContext().messages));
  const stats=session?{...session.getSessionStats(),compaction:{...settings.getCompactionSettings(model),backgroundTokens:0}}:undefined;
  session?.dispose();globalThis.hostInput=undefined;
  __finish(JSON.stringify({ok:ok&&!bridgeFailed&&!stopped,safeError,checkpoint,stats,compactionResult}));
  } catch {session?.dispose();globalThis.hostInput=undefined;__finish(JSON.stringify({ok:false,safeError:'sync_failed'}));}
 }
};
