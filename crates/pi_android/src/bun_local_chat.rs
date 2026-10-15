//! Local UI adapter for the shared Durable runner; SQLite, not this cache, owns execution.
use crate::{app::PhoneApp,model::*,prompt::Prompt,store::Event};
#[cfg(test)] use crate::{app::Route,store::Store};
use gpui::{Context,Window};
use pi_quickjs_backend_poc::{auth::{Provider,Vault},bun::Engine,chat,chat_fixture::Mode};
use serde_json::{Value,json};
use std::{collections::HashMap,path::{Path,PathBuf},sync::{Arc,Mutex,OnceLock,mpsc},time::Duration};
const FEATURES:&[&str]=&["models","thinking","attachments","streaming","stop","visualize","manual_compaction","auto_compaction"];
static ROOT:OnceLock<PathBuf>=OnceLock::new();
pub fn configure(data:&Path){let _=ROOT.set(data.parent().unwrap_or(data).join("no_backup/local-agent"));}
#[derive(Clone,serde::Serialize,serde::Deserialize)]
struct Meta {id:u32,provider:String,model:String,thinking:String,#[serde(default)]pending:Option<Value>,#[serde(default)]attachments:Vec<Vec<String>>}
#[derive(serde::Serialize,serde::Deserialize)]
struct Index {version:u32,next:u32,sessions:Vec<Meta>}
struct Conversation {pi:pi_core::session::Session,engine:Engine,meta:Meta,key:String,cwd:String,status:&'static str,loaded:bool,admitting:bool,compact:Option<String>,stopped:std::cell::Cell<bool>,error:Option<String>,history:Vec<Value>}
pub(crate) struct Local {default_provider:Provider,pub synthetic:bool,vault:Arc<dyn Vault>,default_model:String,default_thinking:String,catalog:Vec<pi_core::protocol::Model>,metadata:Vec<Value>,conversations:HashMap<SessionId,Conversation>,pub mode:Mode,root:PathBuf,next:u32,index_error:Option<String>}
fn response(pi:&mut pi_core::session::Session,command:&str,data:Value)->Result<(),String>{pi.apply(&json!({"type":"response","command":command,"success":true,"data":data})).map_err(|_|"Invalid Local protocol record".into())}
fn provider_id(id:&str)->Option<Provider>{match id{"anthropic"=>Some(Provider::Claude),"openai-codex"=>Some(Provider::Codex),_=>None}}
fn thinking_label(level:&str)->&str{match level{"off"=>"Off","minimal"=>"Minimal","low"=>"Low","medium"=>"Medium","high"=>"High","xhigh"=>"Extra high","max"=>"Max",other=>other}}
impl Local {
 pub fn new(provider:Provider,vault:Arc<dyn Vault>,synthetic:bool)->Self{
  let root=ROOT.get().cloned().unwrap_or_else(||std::env::temp_dir().join("pi-bun-host"));
  let mut metadata=Vec::new();for p in [provider,if provider==Provider::Claude{Provider::Codex}else{Provider::Claude}]{let mut models=chat::catalog(p).as_array().unwrap().clone();models.sort_by_key(|m|m["id"]!=chat::models(p)[0].0);metadata.extend(models);}
  let catalog=serde_json::from_value(json!(metadata)).expect("pinned public catalog");
  let mut result=Self{default_provider:provider,synthetic,vault,default_model:chat::models(provider)[0].0.clone(),default_thinking:"high".into(),catalog,metadata,conversations:HashMap::new(),mode:Mode::Reply,root,next:0x8000_0000,index_error:None};
  match std::fs::read(result.index_path()){
   Ok(bytes)=>match serde_json::from_slice::<Index>(&bytes){Ok(index) if index.version==1&&index.sessions.len()<=8&&index.next>=0x8000_0000=>{result.next=index.next;for meta in index.sessions{if let Err(error)=result.open(meta){result.index_error=Some(error);}}},_=>result.index_error=Some("Saved Local index could not be opened; it was not overwritten".into())},
   Err(error) if error.kind()==std::io::ErrorKind::NotFound=>{},Err(_)=>result.index_error=Some("Saved Local index is unavailable; it was not overwritten".into())
  }result
 }
 fn namespace(&self)->PathBuf{self.root.join(if self.synthetic{"synthetic"}else{"accounts"})}
 fn index_path(&self)->PathBuf{self.namespace().join("index.json")}
 fn save(&self)->Result<(),String>{self.save_extra(None)}
 fn save_extra(&self,extra:Option<&Meta>)->Result<(),String>{
  if let Some(error)=&self.index_error{return Err(error.clone());}
  let save=||->anyhow::Result<()>{use std::io::Write;std::fs::create_dir_all(self.namespace())?;let mut sessions:Vec<_>=self.conversations.values().map(|c|c.meta.clone()).collect();if let Some(meta)=extra{sessions.push(meta.clone());}sessions.sort_by_key(|m|m.id);let bytes=serde_json::to_vec(&Index{version:1,next:self.next,sessions})?;
   let path=self.index_path();let temp=path.with_extension("new");let mut file=std::fs::OpenOptions::new().write(true).create(true).truncate(true).open(&temp)?;file.write_all(&bytes)?;file.sync_all()?;std::fs::rename(temp,path)?;std::fs::File::open(self.namespace())?.sync_all()?;Ok(())};
  save().map_err(|_|"Could not save Local session index; your draft was kept".into())
 }
 fn open(&mut self,meta:Meta)->Result<(),String>{
  let provider=provider_id(&meta.provider).ok_or("Unsupported saved provider")?;
  if !self.catalog.iter().any(|m|m.provider==meta.provider&&m.id==meta.model){return Err("Saved model is not in this build's catalog".into());}
  let key=format!("local-{}",meta.id);let state=self.namespace().join(&key);let cwd=self.namespace().join("workspace");std::fs::create_dir_all(&cwd).map_err(|_|"Local workspace unavailable")?;
  let engine=Engine::open(self.root.clone(),json!({"state":state,"cwd":cwd,"key":key,"synthetic":self.synthetic,"provider":provider.id(),"model":meta.model,"thinking":meta.thinking,"home":self.root.join("home"),"temp":self.root.join("tmp"),"mode":format!("{:?}",self.mode)}),self.vault.clone()).map_err(|_|"Could not start embedded Bun")?;
  let mut pi=pi_core::session::Session::new(cwd.clone());
  let model=self.catalog.iter().find(|m|m.provider==meta.provider&&m.id==meta.model).unwrap();
  response(&mut pi,"get_state",json!({"sessionId":key,"model":model,"thinkingLevel":meta.thinking,"isStreaming":false,"isCompacting":false,"isBashRunning":false,"autoCompactionEnabled":true}))?;
  response(&mut pi,"get_available_models",json!({"models":self.catalog}))?;
  response(&mut pi,"get_commands",json!({"commands":[]}))?;
  if let Some(pending)=&meta.pending{engine.send(pending.clone()).map_err(|_|"Saved prompt could not be checked")?;}
  let admitting=meta.pending.is_some();
  self.conversations.insert(SessionId(meta.id),Conversation{pi,engine,meta,key,cwd:cwd.to_string_lossy().into_owned(),status:"loading",loaded:false,admitting,compact:None,stopped:std::cell::Cell::new(false),error:None,history:Vec::new()});Ok(())
 }
 fn metadata(&self,provider:Provider,model:&str)->&Value{self.metadata.iter().find(|m|m["provider"]==provider.id()&&m["id"]==model).expect("validated model")}
 fn clamp(&self,provider:Provider,model:&str,level:&str)->String{self.metadata(provider,model)["clampedThinkingLevels"][level].as_str().unwrap_or("high").into()}
 fn raw_levels(&self,provider:Provider,model:&str)->Vec<String>{self.metadata(provider,model)["supportedThinkingLevels"].as_array().unwrap().iter().map(|l|l.as_str().unwrap().into()).collect()}
 pub fn restored(&self)->Vec<Session>{let mut ids:Vec<_>=self.conversations.keys().copied().collect();ids.sort_by_key(|id|id.0);ids.into_iter().map(|id|self.project(id)).collect()}
 pub fn create(&mut self)->Result<Session,String>{
  if self.conversations.len()>=8{return Err("Delete an old Local chat first (eight open sessions maximum)".into());}
  if let Some(error)=&self.index_error{return Err(error.clone());}
  let id=self.next;self.next=self.next.checked_add(1).ok_or("Session identity limit")?;
  let meta=Meta{id,provider:self.default_provider.id().into(),model:self.default_model.clone(),thinking:self.default_thinking.clone(),pending:None,attachments:Vec::new()};
  self.save_extra(Some(&meta))?;
  self.open(meta)?;Ok(self.project(SessionId(id)))
 }
 pub fn current_model(&self,id:Option<SessionId>)->Option<&pi_core::protocol::Model>{if let Some(id)=id{return self.conversations.get(&id)?.pi.state.model.as_ref();}self.catalog.iter().find(|m|m.provider==self.default_provider.id()&&m.id==self.default_model)}
 pub fn provider(&self,id:Option<SessionId>)->Provider{self.current_model(id).and_then(|m|provider_id(&m.provider)).unwrap_or(self.default_provider)}
 pub fn model(&self,id:Option<SessionId>)->String{self.current_model(id).map_or_else(||self.default_model.clone(),|m|m.id.clone())}
 pub fn model_settings(&self,id:Option<SessionId>)->(String,String){let model=self.current_model(id).expect("local model");let thinking=id.and_then(|id|self.conversations.get(&id)).map_or(self.default_thinking.as_str(),|c|c.pi.state.thinking_level.as_str());(model.name.clone().unwrap_or_else(||model.id.clone()),thinking_label(thinking).into())}
 pub fn models(&self)->&[pi_core::protocol::Model]{&self.catalog}
 pub fn thinking_levels(&self,id:Option<SessionId>)->Vec<String>{self.raw_levels(self.provider(id),&self.model(id)).iter().map(|l|thinking_label(l).into()).collect()}
 pub fn set_thinking(&mut self,id:Option<SessionId>,label:&str)->Result<(),String>{let level=self.raw_levels(self.provider(id),&self.model(id)).into_iter().find(|l|thinking_label(l)==label).ok_or("Unsupported thinking level")?;
  if let Some(id)=id{let c=self.conversations.get_mut(&id).ok_or("Session unavailable")?;if c.pi.busy()||c.admitting||c.compact.is_some(){return Err("Stop the reply before changing thinking".into());}c.engine.send(json!({"type":"set_thinking_level","id":crate::prompt::request_id(),"level":level})).map_err(|_|"Local command queue unavailable")?;c.pi.apply(&json!({"type":"thinking_level_changed","level":level})).map_err(|_|"Invalid thinking record")?;c.meta.thinking=level;}else{self.default_thinking=level;}self.save()
 }
 pub fn set_model(&mut self,id:Option<SessionId>,provider:&str,model:&str)->Result<(),String>{let selected=self.catalog.iter().find(|m|m.provider==provider&&m.id==model).cloned().ok_or("Model unavailable")?;let chosen=provider_id(provider).ok_or("Provider unavailable")?;let thinking=self.clamp(chosen,model,id.and_then(|id|self.conversations.get(&id)).map_or(&self.default_thinking,|c|&c.pi.state.thinking_level));
  if let Some(id)=id{let c=self.conversations.get_mut(&id).ok_or("Session unavailable")?;if c.pi.busy()||c.admitting||c.compact.is_some(){return Err("Stop the reply before changing models".into());}if !selected.input.iter().any(|s|s=="image")&&c.pi.messages.iter().any(|m|m["role"]=="user"&&m["content"].as_array().is_some_and(|a|a.iter().any(|b|b["type"]=="image"))){return Err("Choose an image-capable model for this chat".into());}c.engine.send(json!({"type":"set_model","id":crate::prompt::request_id(),"provider":provider,"modelId":model})).map_err(|_|"Local command queue unavailable")?;response(&mut c.pi,"set_model",json!(selected))?;c.pi.apply(&json!({"type":"thinking_level_changed","level":thinking})).map_err(|_|"Invalid thinking record")?;c.meta.provider=provider.into();c.meta.model=model.into();c.meta.thinking=thinking;
  }else{self.default_provider=chosen;self.default_model=model.into();self.default_thinking=thinking;}self.save()
 }
 pub fn supports(&self,feature:&str)->bool{FEATURES.contains(&feature)}
 pub fn commands(&self,id:SessionId)->Option<&[pi_core::protocol::SlashCommand]>{self.conversations.get(&id).map(|_|&[][..])}
 pub fn running(&self)->bool{self.conversations.values().any(|c|!c.loaded||c.pi.busy()||c.admitting||c.compact.is_some())}
 pub fn stopping(&self,id:SessionId)->bool{self.conversations.get(&id).is_some_and(|c|c.stopped.get())}
 /// Navigation changes presentation only. Durable sessions continue while parked.
 pub fn leave_except(&self,_shown:Option<SessionId>){}
 /// Explicit application-lifecycle cancellation; never used for backend navigation.
 pub fn cancel(&self){for(id,c)in &self.conversations{if c.pi.busy()||c.admitting||c.compact.is_some(){let _=self.stop(*id);}}}
 pub fn stop(&self,id:SessionId)->Result<(),String>{let c=self.conversations.get(&id).ok_or("Session unavailable")?;c.engine.send(json!({"type":"abort","id":crate::prompt::request_id()})).map_err(|_|"Local stop command unavailable")?;c.stopped.set(true);Ok(())}
 pub fn remove(&mut self,id:SessionId){if let Some(c)=self.conversations.remove(&id){let _=c.engine.send(json!({"type":"abort","id":crate::prompt::request_id()}));drop(c);}if self.save().is_err(){self.index_error=Some("Could not save Local session removal".into());}}
 pub fn send(&mut self,session:&mut Session,prompt:Prompt,attachments:Vec<String>)->Result<(),String>{
  if prompt.message.trim().is_empty()&&prompt.images.is_empty(){return Err("Write a message first".into());}if prompt.message.len()>2*1024*1024||attachments.len()>8||serde_json::to_vec(&prompt.images).map_or(true,|v|v.len()>6*1024*1024){return Err("Local prompt/attachment limit reached; your draft was kept".into());}
  let c=self.conversations.get_mut(&session.id).ok_or("Session unavailable")?;if c.pi.busy()||c.admitting||c.compact.is_some()||c.meta.pending.is_some(){return Err("Wait for admission/reply or restart to check the retained request identity".into());}
  if !prompt.images.is_empty()&&!c.pi.state.model.as_ref().is_some_and(|m|m.input.iter().any(|i|i=="image")){return Err("Choose an image-capable model first".into());}
  let record=json!({"type":"prompt","id":format!("admit-{}",prompt.request_id),"requestId":prompt.request_id,"message":prompt.message,"images":prompt.images});
  let previous=c.meta.clone();c.meta.pending=Some(record.clone());c.meta.attachments.push(attachments);
  if let Err(error)=self.save(){self.conversations.get_mut(&session.id).unwrap().meta=previous;return Err(error);}
  let c=self.conversations.get_mut(&session.id).unwrap();c.engine.send(json!({"type":"phone_fixture_mode","mode":format!("{:?}",self.mode)})).map_err(|_|"Local command queue unavailable")?;
  c.engine.send(record).map_err(|_|"Prompt admission uncertain; it retains the same request identity on reopen")?;c.admitting=true;c.error=None;c.status="preparing";*session=self.project(session.id);Ok(())
 }
 pub fn compact(&mut self,session:&mut Session,instructions:Option<String>)->Result<(),String>{let c=self.conversations.get_mut(&session.id).ok_or("Session unavailable")?;if !c.loaded||c.pi.busy()||c.admitting||c.compact.is_some(){return Err("Wait for the chat to become idle first".into());}let id=crate::prompt::request_id();let mut record=json!({"type":"compact","id":id});if let Some(instructions)=instructions{record["customInstructions"]=json!(instructions);}c.engine.send(record).map_err(|_|"Local compaction command unavailable")?;c.compact=Some(id);c.error=None;c.status="compacting";*session=self.project(session.id);Ok(())}
 pub fn project(&self,id:SessionId)->Session{let c=&self.conversations[&id];
  let pending:Vec<_>=c.meta.pending.as_ref().filter(|_|c.admitting||c.error.is_some()).map(|record|crate::projection::PendingPrompt{text:record["message"].as_str().unwrap_or("").into(),key:None,starts_turn:true}).into_iter().collect();
  let mut session=crate::projection::project(&c.pi,crate::projection::Facts{id,cwd:&c.cwd,folder:c.cwd.clone(),question:None,outbox:&pending,key:&c.key});session.local=true;session.project="Local".into();session.backend_features=Some(FEATURES.iter().map(|s|(*s).into()).collect());session.details.session_file="Saved on this phone · SQLite".into();
  if !c.loaded{session.activity="Opening saved chat…".into();}else if c.stopped.get()&&c.pi.busy(){session.activity="Stopping…".into();}else if c.compact.is_some(){session.activity="Compacting…".into();session.state=State::Working;}else if c.admitting{session.activity="Admitting prompt…".into();session.state=State::Working;}else if !c.pi.busy()&&session.turns.is_empty(){session.activity="Ready · Android sh tools available".into();}
  if let Some(error)=c.error.as_ref().or(self.index_error.as_ref()){session.state=State::Failed;session.failure=Some(error.clone());session.activity=error.clone();}
  for(turn,attachments)in session.turns.iter_mut().zip(&c.meta.attachments){turn.attachments=attachments.clone();}session
 }
 pub fn tick(&mut self,sessions:&mut [Session],_elapsed:Duration)->Vec<Event>{let mut events=Vec::new();let mut save=false;
  for session in sessions{let Some(c)=self.conversations.get_mut(&session.id)else{continue;};let before=session.state;let mut refresh=false;
   for mut record in c.engine.records(){
    let result: anyhow::Result<()>= (||{match record["type"].as_str(){
     Some("durable_state")=>{c.pi=pi_core::durable::project(&record,&c.pi,&c.key)?;c.loaded=true;},
     Some("durable_events")=>{for event in pi_core::durable::events(&record,&c.pi,&c.key)?{c.pi.apply(&event)?;}let state=pi_core::durable::state_update(&c.pi)?;c.pi.apply(&state)?;},
     Some("durable_metadata")=>{for event in pi_core::durable::metadata(&record,&c.pi,&c.key)?{c.pi.apply(&event)?;}},
     Some("durable_subagents")=>{for event in pi_core::durable::subagents(&record,&c.pi,&c.key)?{c.pi.apply(&event)?;}},
     Some("response")=>{
      if record["command"]=="prompt"{c.admitting=false;if record["success"]==true{c.meta.pending=None;save=true;}else{c.error=Some(record["error"].as_str().unwrap_or("Prompt was not admitted; restart to check its original request identity").into());}}
      if record["command"]=="compact"&&c.compact.as_deref()==record["id"].as_str(){c.compact=None;events.push(Event::Compacted(session.id,if record["success"]==true{None}else{Some(record["error"].as_str().unwrap_or("Compaction did not complete").into())}));refresh=true;}
      if record["command"]=="abort"{c.stopped.set(false);}
      if record["command"]=="get_session_stats"{pi_core::durable::discard_stale_stats(&mut record,&c.pi)?;}
      if record["command"]!="get_state"&&record["command"]!="get_messages"{c.pi.apply(&record)?;}
     },
     Some("local_error")=>{c.loaded=true;c.admitting=false;c.error=Some(record["error"].as_str().unwrap_or("Local runtime unavailable").into());if c.compact.take().is_some(){events.push(Event::Compacted(session.id,c.error.clone()));}},_=>{}
    }Ok(())})();if result.is_err(){c.error=Some("Local presentation could not synchronize; saved SDK state was retained".into());c.loaded=true;c.admitting=false;}
   }
   if c.loaded&&!c.pi.busy()&&!c.admitting&&c.compact.is_none(){c.status="idle";if before==State::Working{refresh=true;events.push(Event::Finished(session.id));}}else if c.compact.is_some(){c.status="compacting";}else if c.pi.busy(){c.status="streaming";}
   if refresh{let _=c.engine.send(json!({"type":"get_session_stats","id":crate::prompt::request_id()}));}
   c.history=c.pi.messages.clone();*session=self.project(session.id);
  }
  if save&&self.save().is_err(){self.index_error=Some("Could not update Local admission index; original request IDs were retained".into());}events
 }
}

// Independent coalescing writer: synthetic test geometry/status only, never chat text.
type Publication = (PathBuf, Option<Value>);
struct Telemetry {
    pending: Arc<Mutex<Option<Publication>>>,
    wake: mpsc::SyncSender<()>,
}
fn publish(path: PathBuf, value: Option<Value>) {
    static WRITER: OnceLock<Telemetry> = OnceLock::new();
    let writer = WRITER.get_or_init(|| {
        let pending = Arc::new(Mutex::new(None::<Publication>));
        let work = pending.clone();
        let (wake, rx) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            while rx.recv().is_ok() {
                let item = work.lock().unwrap_or_else(|e| e.into_inner()).take();
                if let Some((path, value)) = item {
                    if let Some(value) = value {
                        if std::fs::create_dir_all(&path).is_ok()
                            && std::fs::write(path.join("ui.new"), value.to_string()).is_ok()
                        {
                            let _ = std::fs::rename(path.join("ui.new"), path.join("ui.json"));
                        }
                    } else {
                        let _ = std::fs::remove_file(path.join("ui.json"));
                    }
                }
            }
        });
        Telemetry { pending, wake }
    });
    *writer.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some((path, value));
    let _ = writer.wake.try_send(());
}

impl PhoneApp {
    #[cfg(test)]
    pub(crate) fn open_local_chat(
        &mut self,
        provider: Provider,
        vault: Arc<dyn Vault>,
        synthetic: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.cancel_connection_attempt();
        if let Some(store) = &self.store
            && let Some(local) = &store.local
            && local.synthetic == synthetic
        {
            if let Some(id) = store.sessions.first().map(|s| s.id) {
                self.show_session(id, window, cx);
                return true;
            }
        }
        if self
            .store
            .as_ref()
            .and_then(|s| s.local.as_ref())
            .is_some_and(Local::running)
        {
            self.notify_user(
                "The previous local reply is still stopping. Please wait.",
                cx,
            );
            return false;
        }
        let mut backend = Local::new(provider, vault, synthetic);
        let Ok(session) = backend.create() else {
            return false;
        };
        let id = session.id;
        let mut store = Store::local(backend);
        store.sessions.push(session);
        self.install_store(store);
        self.paused = false;
        self.playing = false;
        if !synthetic {
            self.local_private = true;
            publish(self.data_dir.join("quickjs-chat-test"), None);
        }
        self.start.update(cx, |composer, cx| {
            composer.clear(cx);
            composer.set_workspace(true);
            composer.use_files(Some(Vec::new()));
            composer.use_commands(Some(Vec::new()));
        });
        self.show_session(id, window, cx);
        true
    }
    pub(crate) fn invalidate_local_account(
        &mut self,
        provider: Provider,
        synthetic: bool,
        cx: &mut Context<Self>,
    ) {
        // Account changes invalidate authorization, not the conversation. Workers
        // admitted under this account are cancelled; lease revocation still owns
        // logout/refresh coordination. An explicit Send is needed after re-login.
        for store in self.store.iter_mut().chain(self.parked_local.iter_mut()) {
            if let Some(backend) = store.local.as_mut().filter(|b| b.synthetic == synthetic) {
                for c in backend.conversations.values_mut() {
                    if provider_id(&c.pi.state.model.as_ref().unwrap().provider) == Some(provider) {
                        let _ = c.engine.send(json!({"type":"abort","id":crate::prompt::request_id()}));
                    }
                }
            }
        }
        cx.notify();
    }
    /// Shared UI test controls are intentionally synthetic-only; no input/credentials in links.
    pub(crate) fn local_fixture_url(
        &mut self,
        url: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if url == "pi://test/local-backend/synthetic" {
            if gpui_android::activity::is_debug_build()
                && !self.local_private
                && self.local_providers.is_synthetic()
            {
                self.select_local_backend(window, cx);
            }
            return true;
        }
        let mode = match url {
            "pi://test/local-chat/reply" => Mode::Reply,
            "pi://test/local-chat/stall" => Mode::Stall,
            "pi://test/local-chat/reject" => Mode::Reject,
            _ => return false,
        };
        if let Some(local) = self.store.as_mut().and_then(|s| s.local.as_mut())
            && local.synthetic
            && !local.running()
        {
            local.mode = mode;
            cx.notify();
        }
        true
    }
    pub(crate) fn tick_local_chat(&mut self, cx: &mut Context<Self>) {
        let Some(store) = &self.store else {
            return;
        };
        let Some(local) = &store.local else {
            return;
        };
        if !local.synthetic || self.local_private {
            return;
        }
        let current = self
            .model_session()
            .and_then(|id| local.conversations.get(&id));
        let session = self.model_session().and_then(|id| store.session(id));
        let Some(geometry) = cx.try_global::<crate::testing::State>() else {
            return;
        };
        let bounds: std::collections::BTreeMap<_, _> = geometry
            .bounds
            .iter()
            .filter_map(|(key, rect)| {
                let name = key
                    .strip_prefix("Name(\"")
                    .and_then(|s| s.strip_suffix("\")"))
                    .unwrap_or(key);
                ([
                    "draft",
                    "send",
                    "back",
                    "composer",
                    "composer-model",
                    "composer-thinking",
                    "model-search",
                    "backend-local",
                    "composer-attach",
                    "attach",
                    "stop",
                    "new-session",
                    "computers",
                    "local-accounts",
                    "search",
                ]
                .contains(&name)
                    || name.starts_with("model-choice-")
                    || name.starts_with("thinking-choice-")
                    || name.starts_with("session-row-")
                    || name.starts_with("thread-"))
                .then_some((name, *rect))
            })
            .collect();
        let visible = self.visible && self.window_active && !self.local_providers.visible;
        publish(
            self.data_dir.join("quickjs-chat-test"),
            Some(
                json!({"synthetic":true,"visible":visible,"pid":std::process::id(),
            "frame":self.local_frame,"route":format!("{:?}",self.route()),"sheet":self.sheet.map(|s|format!("{s:?}")),
            "provider":local.provider(self.model_session()).id(),"model":local.model(self.model_session()),"thinking":local.model_settings(self.model_session()).1,"status":current.map_or("idle",|c|c.status),
            "running":local.running(),"sessions":store.sessions.len(),"turns":session.map_or(0,|s|s.turns.len()),"context_messages":current.map_or(0,|c|c.history.len()),
            "reply_verified":session.and_then(|s|s.turn()).and_then(|t|t.summary.as_ref()).is_some_and(|s|s.text()=="Hello from the phone 🌍"),
            "tool_verified":current.is_some_and(|c|c.pi.tools.iter().any(|tool|tool.finished&&!tool.is_error&&tool.output.contains("phone-tool-ok"))),
            "bounds":if visible {json!(bounds)} else {json!({})},"scale":geometry.scale,"mode":format!("{:?}",local.mode)}),
            ),
        );
        if visible {
            cx.notify();
        }
    }
}

