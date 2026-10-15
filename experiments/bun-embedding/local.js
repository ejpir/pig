// Android in-process transport for the shared desktop Durable runner.
import {BACKGROUND_CONTEXT} from '@earendil-works/chord/context';
import {run} from '../../backend/durable/src/run.ts';
import {NodeExecutionEnv} from '@earendil-works/pi-durable/env/node';
import {phoneModels,installNativeFetch} from './native-models.js';

const sessions=new Map(),pending=new Map(),http=new Map();
const outbox=[];let bytes=0,next=0;
function emit(value){const text=JSON.stringify(value);if(Buffer.byteLength(text)>16*1024*1024||bytes+Buffer.byteLength(text)>64*1024*1024)throw new Error('Native delivery queue limit');outbox.push(text);bytes+=Buffer.byteLength(text);}
function call(engine,type,data){const id=++next;return new Promise((resolve,reject)=>{pending.set(id,{resolve,reject});emit({channel:'host',engine,type,id,...data});});}
const bridge={call,fetch(operation,request,signal){
  signal?.throwIfAborted();const id=++next;
  return new Promise((resolve,reject)=>{
    const state={resolve,reject,controller:null,terminal:false,abort(){
      if(!state.terminal){state.terminal=true;emit({channel:'host',type:'http_cancel',id});try{state.controller?.error(new Error('Request cancelled'));}catch{}reject(new Error('Request cancelled'));}
    }};
    http.set(id,state);signal?.addEventListener('abort',state.abort,{once:true});state.cleanup=()=>signal?.removeEventListener('abort',state.abort);
    emit({channel:'host',type:'http',id,operation,request});
  });
}};
installNativeFetch(bridge);
class Input {
  values=[];waiter=null;closed=false;
  put(value){if(this.closed)throw new Error('Session transport closed');const item=Buffer.from(JSON.stringify(value)+'\n');if(this.waiter){const resolve=this.waiter;this.waiter=null;resolve({value:item,done:false});}else this.values.push(item);}
  close(){this.closed=true;this.waiter?.({done:true});this.waiter=null;}
  [Symbol.asyncIterator](){return this;}
  next(){if(this.values.length)return Promise.resolve({value:this.values.shift(),done:false});return this.closed?Promise.resolve({done:true}):new Promise(resolve=>{this.waiter=resolve;});}
}
globalThis.__piPhone={
  init({home,temp}){process.env.HOME=home;process.env.TMPDIR=temp;},
  open(engine,options){
    if(sessions.has(engine))throw new Error('Duplicate embedded session');
    const input=new Input(),control={mode:options.mode??'Reply'},environments=new Set();
    const session={input,control};sessions.set(engine,session);
    const features=['durable_execution','committed_snapshots','incremental_events','request_deduplication','native_subscription_accounts','image_prompts','image_references','manual_compaction','subagents'];
    const ui=['models','thinking','attachments','streaming','stop','visualize','manual_compaction','auto_compaction'];
    session.running=run(phoneModels(engine,bridge,options.synthetic,control),[],undefined,{
      state:options.state,cwd:options.cwd,key:options.key,input,
      assertOwner:()=>call(engine,'assert_owner',{}),stopInput:()=>input.close(),
      initialModel:{provider:options.provider,modelId:options.model},initialThinking:options.thinking,
      output:line=>emit({channel:'protocol',engine,record:JSON.parse(line)}),
      env:({cwd})=>{const env=new NodeExecutionEnv({cwd:cwd??options.cwd,shellPath:options.shellPath??'/system/bin/sh',shellEnv:{PATH:'/system/bin',HOME:options.home,TMPDIR:options.temp}});environments.add(env);return env;},
      info:{features,uiCapabilities:ui,limitations:['Android sh, not GNU Bash/Linux toolchains','In-process experimental Local backend'],local:true},
      onReport:()=>{},
    }).catch(()=>emit({channel:'protocol',engine,record:{type:'local_error',error:'Local durable session could not continue. State was retained; restart the app to reopen it.'}}))
      .finally(async()=>{await Promise.allSettled([...environments].map(env=>env.cleanup(BACKGROUND_CONTEXT)));sessions.delete(engine);emit({channel:'closed',engine});});
  },
  send(engine,record){const session=sessions.get(engine);if(!session)throw new Error('Local session is closed');if(record.type==='phone_fixture_mode')session.control.mode=record.mode;else session.input.put(record);},
  close(engine){sessions.get(engine)?.input.close();},
  deliver(id,data,error){const item=pending.get(id);if(item){pending.delete(id);error?item.reject(new Error(error)):item.resolve(data);}},
  http(event){const item=http.get(event.id);if(!item)return;
    if(event.kind==='headers')item.resolve(new Response(new ReadableStream({start(controller){item.controller=controller;},cancel(){item.abort();}}),{status:event.status,headers:event.headers}));
    else if(event.kind==='chunk'&&!item.terminal)item.controller?.enqueue(new Uint8Array(event.bytes));
    else if(event.kind==='done'&&!item.terminal){item.terminal=true;item.controller?.close();}
    else if(event.kind==='error'&&!item.terminal){item.terminal=true;item.controller?.error(new Error('Native provider transport failed'));item.reject(new Error('Native provider transport failed'));}
    else if(event.kind==='released'){item.cleanup();http.delete(event.id);}
  },
  drain(){const selected=[];let size=2;while(outbox.length&&size+Buffer.byteLength(outbox[0])+1<22*1024*1024){const value=outbox.shift();bytes-=Buffer.byteLength(value);size+=Buffer.byteLength(value)+1;selected.push(value);}if(outbox.length&&!selected.length)throw new Error('Local presentation record exceeds delivery limit');return '['+selected.join(',')+']';},
};
