export function setTimeout(ms,value,options={}){return new Promise((resolve,reject)=>{
 const signal=options.signal;
 if(signal?.aborted){reject(signal.reason);return;}
 const abort=()=>{clearTimeout(id);signal.removeEventListener('abort',abort);reject(signal.reason);};
 const id=globalThis.setTimeout(()=>{signal?.removeEventListener('abort',abort);resolve(value);},ms);
 signal?.addEventListener('abort',abort,{once:true});
});}
