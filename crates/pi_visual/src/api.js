// Trusted bootstrap, evaluated before guest source; its bridge stays in Rust.
(() => {
  'use strict';
  const parse = JSON.parse, stringify = JSON.stringify;
  const keys = Object.keys, own = Object.getOwnPropertyDescriptor;
  const proto = Object.getPrototypeOf, create = Object.create, freeze = Object.freeze;
  const names = Object.getOwnPropertyNames, symbols = Object.getOwnPropertySymbols;
  const apply = Reflect.apply, textString = String, ErrorType = Error;
  const abs = Math.abs, min = Math.min;
  const fail = message => { throw new ErrorType(message); };
  // Native sparse-array scans can omit VM interrupt polls. Bound their input
  // lengths before entering C; this is still not OS/process isolation.
  const nativeArrayMethods = names(Array.prototype);
  for (const name of nativeArrayMethods) {
    if (name === 'constructor') continue;
    const method = own(Array.prototype,name)?.value;
    if (typeof method !== 'function') continue;
    Object.defineProperty(Array.prototype,name,{value:function(...args){
      const lengthDescriptor = this == null ? undefined : own(this,'length');
      if (!lengthDescriptor || !('value' in lengthDescriptor)) fail('Array operations require a data length');
      const length = lengthDescriptor.value;
      if (!finite(length) || length < 0 || length > 8192) fail('Array operation exceeds 8192 elements');
      if (name === 'concat') {
        const output=[];
        for (const value of [this,...args]) {
          if (!isArray(value)) {output.push(value);continue;}
          const descriptor=own(value,'length');
          if (!descriptor || descriptor.value>8192 || output.length+descriptor.value>8192) fail('Array operation exceeds 8192 elements');
          for(let i=0;i<descriptor.value;i++) output.push(value[i]);
        }
        return output;
      }
      return apply(method,this,args);
    },writable:false,configurable:false});
  }
  const isArray = Array.isArray, finite = Number.isFinite;
  const PromiseType = Promise;
  // Keep serialization/reflection intrinsics immutable. No modules/host APIs exist.
  for (const value of [Object.prototype, Array.prototype, Function.prototype, String.prototype,
    Number.prototype, Boolean.prototype, Object, Array, Function, String, Number, Boolean, Math, JSON, PromiseType, PromiseType.prototype]) freeze(value);
  function data(object, key, fallback) {
    const d = own(object, key);
    if (!d) return fallback;
    if (!('value' in d)) fail('Accessors are not allowed in app data');
    return d.value;
  }
  function flat(value) {
    if (!value || typeof value !== 'object' || isArray(value)) fail('State must be a flat object');
    const fields = names(value); if (fields.length > 64) fail('State exceeds 64 fields');
    if (symbols(value).length) fail('Symbol state fields are not supported');
    const result = create(null);
    for (const key of fields) {
      if (!own(value,key).enumerable) fail('Non-enumerable state fields are not supported');
      const v = data(value,key);
      if (!['number','boolean','string'].includes(typeof v) || (typeof v === 'number' && !finite(v))) fail('State must contain finite primitive values');
      if (typeof v === 'string' && v.length > 256) fail('State string too long');
      if (key.length > 64) fail('State field name too long');
      if (key.startsWith('$text')) fail('State field prefix $text is reserved');
      result[key] = v;
    }
    return result;
  }
  let root, count = 0;
  const handlers = create(null), getters = create(null);
  function register(fn, prefix, id) {
    if (typeof fn !== 'function') fail('Callback must be a function');
    const key = id || prefix + count++;
    if (typeof key !== 'string' || key.length > 32 || key === '__tick' || own(handlers,key)) fail('Invalid or duplicate callback id');
    handlers[key] = fn; return key;
  }
  function text(value, o = {}) {
    const style = data(o,'style','muted');
    if (typeof value === 'function') {
      const key = '$text' + count++; getters[key] = value;
      return {type:'text',text:null,bind:key,style};
    }
    return {type:'text',text:textString(value),bind:null,style};
  }
  const ui = freeze({
    column(children,o={}) { return {type:'column',children,gap:data(o,'gap',8),width:data(o,'width',null)}; },
    row(children,o={}) { return {type:'row',children,gap:data(o,'gap',8)}; },
    text,
    button(label,fn,o={}) { return {type:'button',label,action:register(fn,'button',data(o,'id')),tone:data(o,'tone','normal')}; },
    slider(label,bind,o) { return {type:'slider',label,bind,min:data(o,'min'),max:data(o,'max'),step:data(o,'step',1),
      action:register(data(o,'onChange',()=>{}),'slider',data(o,'id'))}; },
    canvas(o) { return {type:'canvas',label:data(o,'label','Canvas'),height:data(o,'height',240),view_box:data(o,'viewBox',[0,0,100,100]),
      draw:register(data(o,'draw'),'scene',data(o,'id')),
      pointer_action:data(o,'onPointer') ? register(data(o,'onPointer'),'pointer',data(o,'pointerId')) : null}; }
  });
  function app(value) {
    if (root) fail('Register exactly one app');
    if (data(value,'version',3) !== 3) fail('Unsupported visual app version');
    root = {title:data(value,'title'),initial_state:flat(data(value,'state')),view:data(value,'view'),animation:null};
    const animation = data(value,'animation');
    if (animation) {
      handlers.__tick = data(animation,'tick');
      if (typeof handlers.__tick !== 'function') fail('Animation tick must be a function');
      root.animation = {running:data(animation,'running')};
      if (data(animation,'fps') !== undefined) root.animation.fps = data(animation,'fps');
    }
  }
  Object.defineProperties(globalThis,{app:{value:app},ui:{value:ui},fmt:{value:n=>String(n)},finite:{value:finite}});
  function ready() { if (!root) fail('Call app({...}) exactly once'); }
  function sync(result) { if (result instanceof PromiseType) fail('Async callbacks are not supported'); }
  function shapes() {
    const output = []; let vertices = 0;
    function number(n) { if (!finite(n) || abs(n)>1e6) fail('Invalid geometry'); return n; }
    function pair(p) {
      if (!isArray(p) || p.length!==2) fail('Expected [x,y]');
      return [number(data(p,'0')),number(data(p,'1'))];
    }
    function style(o) { return {color:data(o,'color','#70B8FF'),width:data(o,'width',1.5)}; }
    function add(shape,n) {
      vertices += n;
      if (vertices>2048 || output.length>=64) fail('Drawing exceeds geometry limit');
      output.push(shape);
    }
    const g = freeze({
      line(x1,y1,x2,y2,o={}) { add({type:'line',from:[number(x1),number(y1)],to:[number(x2),number(y2)],...style(o)},2); },
      circle(x,y,r,o={}) { add({type:'circle',center:[number(x),number(y)],radius:number(r),fill:data(o,'fill',null),...style(o)},64); },
      rect(x,y,w,h,o={}) { add({type:'rect',origin:[number(x),number(y)],size:[number(w),number(h)],fill:data(o,'fill',null),...style(o)},4); },
      polyline(points,o={}) {
        if (!isArray(points) || points.length<2 || points.length>2048) fail('Polyline must have 2..2048 points');
        // Automatically segment long curves, preserving the joining endpoint.
        for (let start=0;start<points.length-1;start+=63) {
          const chunk=[];for(let i=start;i<min(start+64,points.length);i++) chunk.push(pair(data(points,textString(i))));
          add({type:'polyline',points:chunk,...style(o)},chunk.length);
        }
      }
    });
    return {g,output};
  }
  return freeze({
    definition() { ready();return stringify(root); },
    invoke(mode,id,stateJSON,argsJSON) {
      ready(); const state = flat(parse(stateJSON)), args = parse(argsJSON);
      if (mode==='texts') {
        const results=create(null); const snapshot=freeze(state);
        for (const key of keys(getters)) {
          const result=apply(getters[key],undefined,[snapshot]);sync(result);
          if(typeof result!=='string'||result.length>4096)fail('Text getter must return a bounded string');
          results[key]=result;
        }
        return stringify(results);
      }
      if (mode==='text') {
        if (typeof getters[id] !== 'function') fail('Unknown text getter');
        const result = apply(getters[id],undefined,[freeze(state)]);sync(result);
        if (typeof result !== 'string' || result.length>4096) fail('Text getter must return a bounded string');
        return stringify(result);
      }
      if (typeof handlers[id] !== 'function') fail('Unknown callback');
      if (mode==='draw') {
        const {g,output} = shapes();sync(apply(handlers[id],undefined,[g,freeze(state)]));
        return stringify(output);
      }
      sync(apply(handlers[id],undefined,[state,...args]));
      return stringify(flat(state));
    }
  });
})()
