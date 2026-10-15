// Public provider wrappers; native code remains the sole OAuth/HTTP owner.
import {createModels} from '@earendil-works/pi-ai/models';
import {InMemoryCredentialStore,lazyStream,isContextOverflow} from '@earendil-works/pi-ai';
import {anthropicProvider} from '@earendil-works/pi-ai/providers/anthropic';
import {openaiCodexProvider} from '@earendil-works/pi-ai/providers/openai-codex';
import {fauxProvider,fauxAssistantMessage,fauxToolCall} from '@earendil-works/pi-ai/providers/faux';
import {durableModels} from '../../backend/durable/src/model-stream.ts';

const locks=new Map();
async function acquire(provider,signal) {
  signal?.throwIfAborted();
  let release;const done=new Promise(resolve=>{release=resolve;});
  const previous=locks.get(provider)??Promise.resolve();
  locks.set(provider,previous.then(()=>done));
  let abort;
  try {await Promise.race([previous,new Promise((_,reject)=>{abort=()=>reject(new Error('Request cancelled'));signal?.addEventListener('abort',abort,{once:true});if(signal?.aborted)abort();})]);}
  catch(error){release();throw error;}finally{signal?.removeEventListener('abort',abort);}
  if(signal?.aborted){release();signal.throwIfAborted();}
  return release;
}
export function phoneModels(engine,bridge,synthetic,control) {
  const models=createModels({credentials:new InMemoryCredentialStore(),authContext:{env:async()=>undefined,fileExists:async()=>false}});
  for(const stock of [anthropicProvider(),openaiCodexProvider()]) {
    const faux=fauxProvider({tokensPerSecond:180});
    faux.setResponses(Array.from({length:10000},()=>async({messages})=>{
      const user=messages.findLast(message=>message.role==='user');
      const text=typeof user?.content==='string'?user.content:(user?.content??[]).filter(b=>b.type==='text').map(b=>b.text).join('\n');
      if(control.mode==='Reject')throw new Error('Synthetic provider rejection');
      if(text.includes('offline tools')) {
        const result=messages.slice(messages.findLastIndex(message=>message.role==='user')+1).findLast(message=>message.role==='toolResult');
        if(!result)return fauxAssistantMessage([fauxToolCall('bash',{command:text.includes('slow')?"printf started > navigation-tool-started; sleep 8; printf finished > navigation-tool-finished; printf 'phone-tool-ok\\n'":"printf 'phone-tool-ok\\n'",timeout:15},{id:'phone-tool-'+Date.now()})],{stopReason:'toolUse'});
        return fauxAssistantMessage('Tool completed: '+result.content.filter(b=>b.type==='text').map(b=>b.text).join(''));
      }
      return fauxAssistantMessage(control.mode==='Stall'?'Waiting '.repeat(10000):'Hello from the phone 🌍');
    }));
    const wrap=invoke=>(model,context,options={})=>lazyStream(model,async()=>{
      if(synthetic)return invoke===stock.stream?faux.provider.stream(model,context,options):faux.provider.streamSimple(model,context,options);
      const unlock=await acquire(stock.id,options.signal);
      let ticket;
      try {
        ticket=await bridge.call(engine,'prepare',{provider:stock.id});
        options.signal?.throwIfAborted();
        if(ticket.resolved?.source!=='OAuth'||!ticket.resolved.auth)throw new Error('Account authorization unavailable');
        const resolved=ticket.resolved;
        const controller=new AbortController(),abort=()=>controller.abort();options.signal?.addEventListener('abort',abort,{once:true});if(options.signal?.aborted)abort();
        const requestModel=resolved.auth.baseUrl?{...model,baseUrl:resolved.auth.baseUrl}:model;
        const stream=invoke(requestModel,context,{...options,signal:controller.signal,apiKey:resolved.auth.apiKey,
          headers:{...resolved.auth.headers,...options.headers,'x-pi-native-operation':String(ticket.operation)},
          env:resolved.env,
          transport:'sse',maxRetries:0,timeoutMs:180000,maxTokens:options.maxTokens??4096});
        return (async function*(){
          let terminal;
          try {
            for await(const event of stream) {
              const frame=event.partial??event.message??event.error;
              if(frame&&Buffer.byteLength(JSON.stringify(frame))>512*1024){controller.abort();throw new Error('Assistant output limit');}
              const clean=event.type==='error'?{...event,error:{...event.error,errorMessage:isContextOverflow(event.error,model.contextWindow)?'context length exceeded':'Provider request did not complete. Check Local accounts or connectivity.'}}:event;
              if(event.type==='done'||event.type==='error')terminal=clean;else yield clean;
            }
          } finally {
            controller.abort();
            try{await bridge.call(engine,'release',{operation:ticket.operation});}finally{ticket.resolved=null;options.signal?.removeEventListener('abort',abort);unlock();}
          }
          // Durable can start another model request immediately on this event.
          // Native release confirms all HTTP runtimes/sockets have torn down first.
          if(terminal)yield terminal;
        })();
      } catch(error) {
        if(ticket)await bridge.call(engine,'release',{operation:ticket.operation}).catch(()=>{});
        unlock();throw new Error(options.signal?.aborted?'Request cancelled':'Local account/provider request could not start');
      }
    });
    models.setProvider({id:stock.id,name:stock.name,baseUrl:stock.baseUrl,headers:stock.headers,getModels:()=>stock.getModels(),
      auth:{apiKey:{name:'Native subscription account',
        check:async()=>synthetic||await bridge.call(engine,'account_status',{provider:stock.id})?{type:'oauth',source:'OAuth'}:undefined,
        resolve:async()=>({auth:{},source:'OAuth'})}},
      stream:wrap(stock.stream),streamSimple:wrap(stock.streamSimple)});
  }
  return durableModels(models);
}

export function installNativeFetch(bridge) {
  globalThis.fetch=async(input,options={})=>{
    const request=input instanceof Request?input:null;
    const headers=new Headers(options.headers??request?.headers);
    const operation=Number(headers.get('x-pi-native-operation'));headers.delete('x-pi-native-operation');
    if(!Number.isSafeInteger(operation)||operation<=0)throw new Error('Native provider transport admission required');
    const body=options.body??(request?new Uint8Array(await request.arrayBuffer()):'');
    const binary=body instanceof ArrayBuffer?new Uint8Array(body):ArrayBuffer.isView(body)?new Uint8Array(body.buffer,body.byteOffset,body.byteLength):null;
    if((typeof body!=='string'&&!binary)||(binary?.byteLength??Buffer.byteLength(body))>10*1024*1024)throw new Error('Native request body limit');
    return bridge.fetch(operation,{url:request?.url??String(input),method:options.method??request?.method??'GET',headers:Object.fromEntries(headers),...(binary?{bodyBase64:Buffer.from(binary).toString('base64')}:{body})},options.signal??request?.signal);
  };
}
