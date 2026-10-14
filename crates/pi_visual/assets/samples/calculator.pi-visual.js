function reset(s){Object.assign(s,{entry:'0',display:'0',value:0,lhs:0,op:'',mode:'entry',caption:''});return s;}
function symbol(op){return {'*':'×','/':'÷','-':'−'}[op]||op;}
function arithmetic(a,b,op){if(op==='/'&&b===0)throw Error('Cannot divide by zero');const r=op==='+'?a+b:op==='-'?a-b:op==='*'?a*b:a/b;if(!Number.isFinite(r))throw Error('Result out of range');return r;}
function refresh(s){s.display=s.mode==='entry'?s.entry:String(Number(s.value.toPrecision(12)));return s;}
function dispatch(s,key){
 const digits='0123456789',operators=['+','-','*','/'];
 if(key==='AC'){reset(s);return;}
 if(s.mode==='error')reset(s);
 try{
  if(digits.includes(key)&&key.length===1||key==='.'){
   if(s.mode!=='entry'){s.entry='0';s.value=0;s.mode='entry';if(s.op==='')s.caption='';}
   if(key==='.'){if(!s.entry.includes('.'))s.entry+='.';}
   else if(s.entry.length<15){s.entry=s.entry==='0'?key:s.entry==='-0'?'-'+key:s.entry+key;}
   s.value=Number(s.entry);
  }else if(key==='backspace'){
   if(s.mode==='entry'){s.entry=s.entry.slice(0,-1);if(s.entry===''||s.entry==='-')s.entry='0';s.value=Number(s.entry);}
  }else if(key==='sign'){
   if(s.mode==='pending'){s.entry='-0';s.value=0;s.mode='entry';}
   else if(s.mode==='entry'){s.entry=s.entry.startsWith('-')?s.entry.slice(1):'-'+s.entry;s.value=Number(s.entry);}
   else s.value=-s.value;
  }else if(key==='%'){if(s.mode==='pending')s.value=0;s.value/=100;s.mode='result';}
  else if(operators.includes(key)){
   if(s.op!==''&&s.mode!=='pending')s.value=arithmetic(s.lhs,s.value,s.op);
   s.lhs=s.value;s.op=key;s.mode='pending';s.caption=fmt(s.value)+' '+symbol(key);
  }else if(key==='='){
   if(s.op===''||s.mode==='pending')return;
   const rhs=s.value;s.value=arithmetic(s.lhs,rhs,s.op);
   s.caption=fmt(s.lhs)+' '+symbol(s.op)+' '+fmt(rhs)+' =';
   s.history3=s.history2;s.history2=s.history1==='No calculations yet'?'':s.history1;
   s.history1=s.caption+' '+fmt(s.value);s.op='';s.mode='result';
  }
  refresh(s);
 }catch(e){reset(s);s.mode='error';s.display='Error';s.caption=e.message;}
}

app({title:"Calculator",state:{"entry": "0", "display": "0", "value": 0.0, "lhs": 0.0, "op": "", "mode": "entry", "caption": "", "history1": "No calculations yet", "history2": "", "history3": ""},view:ui.column([ui.text(s=>String(s["caption"]), {"style": "expression"}),
ui.text(s=>String(s["display"]), {"style": "display"}),
ui.column([ui.row([ui.button("AC",s=>{dispatch(s,"AC");},{"tone": "utility", "id": "AC"}),
ui.button("\u00b1",s=>{dispatch(s,"sign");},{"tone": "utility", "id": "sign"}),
ui.button("%",s=>{dispatch(s,"%");},{"tone": "utility", "id": "%"}),
ui.button("\u00f7",s=>{dispatch(s,"/");},{"tone": "operator", "id": "/"})], {"gap": 6}),
ui.row([ui.button("7",s=>{dispatch(s,"7");},{"tone": "normal", "id": "7"}),
ui.button("8",s=>{dispatch(s,"8");},{"tone": "normal", "id": "8"}),
ui.button("9",s=>{dispatch(s,"9");},{"tone": "normal", "id": "9"}),
ui.button("\u00d7",s=>{dispatch(s,"*");},{"tone": "operator", "id": "*"})], {"gap": 6}),
ui.row([ui.button("4",s=>{dispatch(s,"4");},{"tone": "normal", "id": "4"}),
ui.button("5",s=>{dispatch(s,"5");},{"tone": "normal", "id": "5"}),
ui.button("6",s=>{dispatch(s,"6");},{"tone": "normal", "id": "6"}),
ui.button("\u2212",s=>{dispatch(s,"-");},{"tone": "operator", "id": "-"})], {"gap": 6}),
ui.row([ui.button("1",s=>{dispatch(s,"1");},{"tone": "normal", "id": "1"}),
ui.button("2",s=>{dispatch(s,"2");},{"tone": "normal", "id": "2"}),
ui.button("3",s=>{dispatch(s,"3");},{"tone": "normal", "id": "3"}),
ui.button("+",s=>{dispatch(s,"+");},{"tone": "operator", "id": "+"})], {"gap": 6}),
ui.row([ui.button("\u232b",s=>{dispatch(s,"backspace");},{"tone": "utility", "id": "backspace"}),
ui.button("0",s=>{dispatch(s,"0");},{"tone": "normal", "id": "0"}),
ui.button(".",s=>{dispatch(s,".");},{"tone": "normal", "id": "."}),
ui.button("=",s=>{dispatch(s,"=");},{"tone": "accent", "id": "="})], {"gap": 6})], {"gap": 6, "width": null})], {"gap": 8, "width": null})});
