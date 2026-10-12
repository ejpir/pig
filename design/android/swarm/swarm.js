'use strict';
// All state below is fictitious and memory-only. No fetch, native bridge, storage or Pi calls.
const icons = {
  back:'<path d="m14 6-6 6 6 6M8 12h12"/>', close:'<path d="m6 6 12 12M6 18 18 6"/>',
  more:'<circle cx="12" cy="5" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="12" cy="19" r="1"/>',
  wifi:'<path d="M3 9a15 15 0 0 1 18 0M6 12a10 10 0 0 1 12 0M9 15a5 5 0 0 1 6 0"/><circle cx="12" cy="18" r="1"/>',
  battery:'<rect x="3" y="7" width="17" height="10" rx="2"/><path d="M22 10v4M6 10h10v4H6z"/>',
  arrow:'<path d="M5 12h14m-5-5 5 5-5 5"/>', file:'<path d="M14 3H5v18h14V8zM14 3v5h5M8 12h8M8 16h6"/>',
  check:'<path d="m5 12 4 4L19 6"/>', stop:'<rect x="6" y="6" width="12" height="12" rx="2"/>',
  message:'<path d="M4 4h16v12H9l-5 4zM8 8h8M8 12h5"/>', chevron:'<path d="m9 6 6 6-6 6"/>',
  computer:'<rect x="3" y="4" width="18" height="13" rx="2"/><path d="M8 21h8M12 17v4"/>',
};
const icon = name => `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${icons[name] || icons.message}</svg>`;
const escapeHTML = text => String(text).replace(/[&<>"']/g, ch => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[ch]));
const initialState = () => ({ route:'map', scene:'live', connected:true, lifecycle:'active', sheet:null, agent:'api', filter:'all', choice:null, answer:null, stopped:new Set(), stopping:null, allStopped:false, replied:false, draft:'', selectedGroup:null, previous:'map' });
function fixture(route) {
  const sample=initialState(); sample.route=route;
  if(route==='offline') { sample.connected=false; sample.scene='offline'; }
  if(route==='many') sample.scene='many';
  if(route==='question') { sample.scene='decision'; sample.replied=true; }
  if(route==='done') { sample.scene='finished'; sample.lifecycle='finished'; sample.replied=true; }
  return sample;
}
const closedExchange = s => s.allStopped || s.stopped.has('api') || s.stopped.has('ui') || s.stopping==='all' || ['api','ui'].includes(s.stopping);
let state = initialState();
let toastTimer;
const definitions = {
  pi:{name:'Pi', role:'Coordinator', hue:'accent', task:'Coordinate the swarm, resolve conflicting plans and report the outcome.', model:'Sonnet · session model', tools:'read · write · edit · bash', path:'Integration worktree'},
  scout:{name:'Scout', role:'Research', hue:'read', task:'Find every client of the existing session API. Share the compatibility constraints with the API agent.', model:'Haiku · agent preset', tools:'read · bash', path:'Read-only project view'},
  api:{name:'API', role:'Implementation', hue:'edit', task:'Add cursor-based updates. Keep v1 clients working; agree the response shape with UI before editing.', model:'Sonnet · session model', tools:'read · write · edit · bash', path:'Isolated worktree · api-03'},
  ui:{name:'UI', role:'Implementation', hue:'accent', task:'Add the incremental update view. Agree the cursor contract with API; publish the UI patch for review.', model:'Sonnet · session model', tools:'read · write · edit · bash', path:'Isolated worktree · ui-04'},
  reviewer:{name:'Reviewer', role:'Verification', hue:'green', task:'Review API and UI patches at their exact artifact revisions. Run checks after Pi integrates them.', model:'Sonnet · agent preset', tools:'read · bash', path:'Read-only review worktree'},
};
const views = [
  ['01-live-map','map','A living map','Logical peer exchanges, not network traffic. A waiting API agent is distinct from a question that needs you.'],
  ['02-exchanges','messages','Exchanges, not transcripts','Questions, findings and artifact handoffs retain sender, recipient, time and delivery state.'],
  ['03-agent','agent','One agent, in context','Its brief, tools, isolated workspace and exact blocker. Inspect a conversation or stop just this agent.'],
  ['04-decision','question','Only the decision reaches you','Pi escalates a compatibility choice. Selecting an answer is not sending it; Answer is explicit.'],
  ['05-large-swarm','many','Twenty-four, without the tangle','Group by responsibility. See capacity and spend, then inspect every member or find a group by name.'],
  ['06-offline','offline','Last seen is not live','Freeze the observed state when SSH drops. Do not claim agents stopped, and do not queue hidden actions.'],
  ['07-finished','done','Finished is not integrated','Evidence, check results and patch revisions. Finished agents do not mean changes were applied to main.'],
  ['08-evening','map','Evening','Same map and labels in the dark palette. No blinking, force-layout drift or color-only status.','evening'],
];
function badge(text, kind='') { return `<span class="badge ${kind}">${escapeHTML(text)}</span>`; }
function tabs(route) { return `<nav class="tabs" aria-label="Swarm views">${[['map','Map'],['messages','Exchanges'],['agents','Agents']].map(([key,label]) => `<button data-route="${key}" class="${route===key?'active':''}" aria-current="${route===key?'page':'false'}">${label}</button>`).join('')}</nav>`; }
function agentStatus(key, s, done=false) {
  if (done || s.lifecycle==='finished' || key==='scout') return 'Done';
  if (s.stopping==='all' || s.stopping===key) return 'Stopping';
  if (s.allStopped || s.stopped.has(key)) return 'Stopped';
  if (key==='api' && !s.replied) return 'Waiting';
  if (key==='reviewer') return 'Waiting';
  return 'Working';
}
function agentRow(key,s,done=false) {
  const d=definitions[key], status=agentStatus(key,s,done);
  const note={pi:'Coordinating the next handoff', scout:'Shared compatibility findings',api:s.replied?'Implementing the agreed cursor':'Waiting for UI’s cursor decision',ui:'Comparing cursor formats',reviewer:'Waiting for patch artifacts'}[key];
  return `<button class="agent-row" data-agent="${key}"><span class="tile" style="--role:var(--${d.hue})">${key==='pi'?'π':d.name.slice(0,2)}</span><span class="agent-text"><b>${d.name}</b><span class="meta">${status==='Stopped'?'Mailbox closed; no new work':done?'Delegated work finished':note}</span></span>${badge(status,status==='Waiting'?'wait':status==='Done'?'good':status==='Stopped'?'stale':'')}${icon('chevron')}</button>`;
}
function graph(s,offline=false,done=false) {
  const n=(key) => {
    const d=definitions[key], status=agentStatus(key,s,done);
    return `<button class="node ${key} ${status==='Waiting'?'waiting':status==='Working'?'live':''}" data-agent="${key}" aria-label="${d.name}: ${offline?'last seen ':''}${status}"><b><span class="role-dot" style="--role:var(--${d.hue})"></span>${d.name}</b>${key==='pi'?'<span class="node-role">Coordinator</span>':''}<small>${status==='Done'?'✓ Done':status==='Waiting'?'◷ Waiting':status==='Stopped'?'■ Stopped':status==='Stopping'?'◷ Stopping':'● Working'}</small></button>`;
  };
  const marker=`arrow-${s.uid||'sample'}`;
  return `<div class="map-card"><div class="map-heading">${done?'Final handoffs':offline?'Last observed map':'Session API update'}<span>4 peers + Pi</span></div><div class="graph" aria-label="Scout shared findings with API. API asks UI about the cursor format. Reviewer waits for UI’s patch."><svg viewBox="0 0 372 354" preserveAspectRatio="none" aria-hidden="true"><defs><marker id="${marker}" viewBox="0 0 8 8" refX="7" refY="4" markerWidth="5" markerHeight="5" orient="auto-start-reverse"><path d="M0 0 8 4 0 8Z" fill="currentColor" style="color:var(--read)"/></marker></defs><path class="assignment" d="M150 61V81H87V104M222 61V81H285V104"/><path class="edge" d="M87 179V239" marker-end="url(#${marker})"/><path class="edge ${!s.replied&&!done?'waiting-edge':''}" d="M161 291H210"/><path class="edge ${done?'':'dependency'}" d="M285 245V182" ${done?`marker-end="url(#${marker})"`:''}/></svg>${['pi','scout','reviewer','api','ui'].map(n).join('')}<span class="graph-note">Finding #12 ↓</span><span class="graph-note right">${done?'Patch #19 ↑':'Needs patch ↑'}</span><button class="edge-button" data-action="exchange" aria-label="Inspect API to UI question 13"><span>${done||s.replied?'✓':'?'}</span></button></div><div class="legend"><span><i></i>Peer exchange</span><span><i class="dashed"></i>Assignment / dependency</span></div></div>`;
}
function mapView(s,offline=false,done=false) {
  const stopped=s.allStopped;
  done=done||s.lifecycle==='finished';
  const counts=Object.keys(definitions).reduce((all,key)=> { const status=agentStatus(key,s,done); all[status]=(all[status]||0)+1; return all; },{});
  return `<div class="row between"><span class="label">Swarm / s-84</span>${badge(offline?'Last seen 09:42':done?'Finished':s.stopping?'Stopping':stopped?'Stopped':'Live',offline||stopped?'stale':done?'good':'')}</div><h2 style="margin-top:10px">${done?'Work handed back.':stopped?'The swarm has stopped.':'Parallel work.<br>Shared understanding.'}</h2><div class="summary">${done?'<span><b>4</b> peers finished</span><span><b>2</b> patches</span>':stopped?'<span><b>4</b> stopped incl. Pi</span><span><b>1</b> already done</span>':`<span><b>${counts.Working||0}</b> working incl. Pi</span><span><b>${counts.Waiting||0}</b> waiting</span><span><b>${counts.Done||0}</b> done</span>${counts.Stopped?`<span><b>${counts.Stopped}</b> stopped</span>`:''}${counts.Stopping?`<span><b>${counts.Stopping}</b> stopping</span>`:''}`}</div>${tabs('map')}${graph(s,offline,done)}${offline?'<div class="callout wait"><b>Can’t reach studio-mac</b>Agents may still be working. These counts are from 09:42:18, not the computer’s current state.</div>':stopped?'<div class="callout"><b>Stop acknowledged by studio-mac</b>Pending deliveries cancelled in this sample. Existing edits and artifacts are kept.</div>':done?'<div class="callout"><b>Not applied to the main workspace</b>Pi produced integration candidate c7e1. Review the observed changes before choosing what happens next.</div>':`<div class="callout ${s.replied&&!closedExchange(s)?'':'wait'}"><b>${closedExchange(s)?'Peer Stop closes the pending wait':s.replied?'Cursor contract agreed':'API is waiting on UI, not on you'}</b>${closedExchange(s)?'Pi is notified. Historical receipts are kept; no new reply can wake a stopped assignment.':s.replied?'Reply #14 unblocked API. Reviewer still needs both patch revisions.':'“Can the client persist an opaque cursor?”'}<button class="text-link" data-action="exchange">Inspect exchange ${icon('arrow')}</button></div>`}<div class="activity"><div class="section-label">Recent handoff <span>09:42</span></div><div class="activity-line"><span class="tile" style="--role:var(--read)">Sc</span><div><b>Scout → API</b><p class="meta">3 compatibility constraints · delivered</p><button class="text-link" data-action="artifact">clients.md · revision a91f ${icon('file')}</button></div></div></div>`;
}
const messageRecords = [
  {id:13,from:'API',to:'UI',kind:'Question',text:'Can the client persist an opaque cursor? I need the response contract before I edit.',time:'09:42:16',status:'Delivered · awaiting reply',waiting:true},
  {id:12,from:'Scout',to:'API',kind:'Finding',text:'Three v1 callers expect a full snapshot. Keep the compatibility path.',time:'09:42:03',status:'Delivered to context',artifact:'clients.md · a91f'},
  {id:11,from:'Pi',to:'Reviewer',kind:'Assignment',text:'Review both patch revisions, then check the integration candidate.',time:'09:41:52',status:'Delivered · waiting for artifacts'},
];
function messagesView(s) {
  const records=[...(s.replied?[{id:14,from:'UI',to:'API',kind:'Reply',text:'Yes. Persist it per session; reset it when the snapshot version changes.',time:'09:42:25',status:'Delivered · resolves #13'}]:[]),...messageRecords];
  const shown=records.filter(m=>s.filter==='all'||(s.filter==='questions'&&(m.kind==='Question'||m.kind==='Reply'))||(s.filter==='artifacts'&&m.artifact));
  return `<span class="label">Swarm / s-84</span><h2 style="margin-top:10px">What they shared.</h2><p class="meta">Peer messages, not private reasoning or full transcripts.</p>${tabs('messages')}<div class="filter-row" aria-label="Filter exchanges">${[['all','All'],['questions','Questions'],['artifacts','Artifacts']].map(([key,text])=>`<button data-filter="${key}" class="${s.filter===key?'active':''}" aria-pressed="${s.filter===key}">${text}</button>`).join('')}</div>${shown.map(m=>`<button class="exchange" data-action="${m.id===12?'artifact':m.id===11?'assignment':'exchange'}"><div class="route"><span>${m.from} → ${m.to}</span>${badge(m.kind,m.waiting&&!s.replied?'wait':'')}</div><p>${m.text}</p>${m.artifact?'<span class="artifact">'+icon('file')+'clients.md <span class="meta">a91f · immutable</span></span>':''}<div class="meta" style="margin-top:8px">#${m.id} · ${m.time} · ${m.id===13?closedExchange(s)?s.replied?'Answered before Stop · mailbox now closed':'Wait cancelled by Stop':s.replied?'Answered by #14':m.status:m.status}</div></button>`).join('')}<div class="callout"><b>A receipt is not an agreement</b>Delivered means admitted to the recipient’s context. Replies and completed work have their own events.</div>`;
}
function agentView(s) {
  const key=s.agent,d=definitions[key],status=agentStatus(key,s);
  return `<div class="row between"><span class="label">Peer / ${key==='api'?'api-03':key+'-04'}</span>${badge(status,status==='Waiting'?'wait':status==='Done'?'good':status==='Stopped'?'stale':'')}</div><h2 style="margin-top:12px">${d.name}</h2><p class="meta">${d.role} · belongs to “Session API update”</p><div class="detail-brief">${d.task}</div><dl class="facts"><div><dt>Model</dt><dd>${d.model}</dd></div><div><dt>Observed spend</dt><dd>${key==='api'?'$0.14':'$0.08'} / 2m 18s</dd></div><div><dt>Tools</dt><dd>${d.tools}</dd></div><div><dt>Workspace</dt><dd>${d.path}</dd></div></dl>${key==='api'?`<div class="callout wait"><b>${status==='Stopped'?'This agent has stopped':status==='Stopping'?'Stop requested, not acknowledged':closedExchange(s)?'UI’s mailbox has closed':s.replied?'Reply received from UI':'Waiting for UI’s reply'}</b>${status==='Stopped'?'New messages are refused; the original wait was cancelled.':status==='Stopping'?'The computer has not acknowledged Stop yet. Effects already in flight may complete.':closedExchange(s)?'The question ended with peer-stopped. Pi must resolve the remaining dependency.':s.replied?'Reply #14 refers to question #13. API is now free to continue.':'Question #13 · expires after 2 minutes. Pi is notified if the peer fails or the wait expires.'}<button class="text-link" data-action="exchange">Open API ↔ UI ${icon('arrow')}</button></div>`:'<div class="callout"><b>Isolated conversation</b>Only explicitly addressed messages enter this agent’s context. A peer message cannot grant new tools or permissions.</div>'}<div class="section-label">Shared with this agent</div><button class="artifact" data-action="artifact">${icon('file')}<span><b>clients.md</b><br><span class="meta">Scout · revision a91f · read-only</span></span>${icon('chevron')}</button><div class="section-label">Latest observed activity</div><div class="activity-line"><time>09:42:18</time><div><b>${status==='Stopped'?'Stop acknowledged':key==='api'?'Read src/protocol.ts':'Updated progress'}</b><p class="meta">${status==='Stopped'?'No later command started in this sample.':'No new file edits reported.'}</p></div></div><button class="text-link" data-action="transcript">Open this agent’s conversation ${icon('arrow')}</button>`;
}
function questionView(s) {
  return `<div class="row between"><span class="label">Pi / escalated from API</span>${badge(s.answer?'Answered':'Needs you',s.answer?'good':'wait')}</div><h2 style="margin-top:14px">Keep older clients<br>working?</h2><p class="secondary">API and UI agree on an opaque cursor. Pi needs your choice before changing the v1 compatibility path.</p><div class="callout"><b>What this unblocks</b>API’s compatibility patch, then Reviewer’s integration check. Scout’s findings and other work are kept.</div><div class="section-label">Your decision</div>${s.answer?`<div class="card"><b>${s.answer==='keep'?'Keep v1 compatibility':'Allow a v2-only change'}</b><p class="meta" style="margin-top:8px">Sample answer acknowledged. Pi would forward a scoped decision; no real request was sent.</p></div>`:[['keep','Keep v1 compatibility','Add the cursor path without removing snapshots.'],['v2','Allow a v2-only change','Older clients will need an update.']].map(([key,title,note])=>`<button class="question-choice" data-choice="${key}" aria-pressed="${s.choice===key}"><span class="radio"></span><span><b>${title}</b><br><span class="meta">${note}</span></span></button>`).join('')}<div class="section-label">How it reached you</div><div class="activity-line"><time>09:42</time><div><b>API → Pi</b><p class="meta">Compatibility policy is outside the delegated brief.</p></div></div><div class="activity-line"><time>09:43</time><div><b>Pi → You</b><p class="meta">One consolidated decision, not every peer message.</p></div></div><p class="meta" style="margin-top:20px">Picking an option does not send it. Later leaves this decision waiting.</p>`;
}
const groups=[['API',6,2,1,3,'edit'],['UI',6,3,2,1,'accent'],['Tests',8,2,1,5,'read'],['Review',4,1,1,2,'green']];
function manyView(s) {
  return `<div class="row between"><span class="label">Swarm / s-91</span>${badge('Live')}</div><h2 style="margin-top:12px">Many agents.<br>Four responsibilities.</h2><div class="summary"><span><b>8</b> working</span><span><b>5</b> waiting</span><span><b>11</b> done</span></div><div class="callout"><div class="row between"><b>8 / 8 peer slots</b><span class="meta">24 peers total</span></div><p class="meta">Waiting agents release their slot. Messages do not create more agents.</p></div><div class="group-grid">${groups.map(([name,total,work,wait,done,hue])=>`<button class="group-card" data-group="${name}"><div class="row between"><b>${name}</b>${badge(total)}</div><small>${work} working · ${wait} waiting<br>${done} done</small><div class="mini-tiles" style="--role:var(--${hue})">${Array.from({length:total},()=>'<i></i>').join('')}</div></button>`).join('')}</div><div class="card"><div class="row between"><b style="font-size:13px">$0.96 observed spend</b><span class="meta">$3.00 cap</span></div><div class="budget"><span></span></div><p class="meta" style="margin-top:10px">A server-side admission limit, not a guarantee of exact final billing. Group details include every agent.</p></div><div class="section-label">One blocker needs attention ${badge('Waiting','wait')}</div><button class="exchange" data-action="large-blocker"><div class="route">UI-04 → API-02</div><p>Waiting for the schema revision; Tests-03 also depends on it.</p><div class="meta">Open the blocker and its affected peers →</div></button><button class="text-link" data-action="find">Find a group by name ${icon('arrow')}</button>`;
}
function doneView(s) {
  return `<div class="row between"><span class="label">Swarm / s-84</span>${badge('Finished','good')}</div><h2 style="margin-top:12px">Four peers.<br>One candidate.</h2><p class="secondary">The cursor path is ready to review. API and UI exchanged a contract, then Reviewer checked their integration candidate.</p><div class="card" style="margin-top:18px"><div class="row between"><b style="font-size:13px">Integration check</b>${badge('Passed','good')}</div><p class="meta" style="margin-top:8px">bun test test/cursor.test.ts<br>32 passed · exit 0 · candidate c7e1</p></div><div class="section-label">Evidence & handoffs <span>5 exchanges</span></div><button class="artifact" data-action="artifact">${icon('file')}<span><b>Compatibility findings</b><br><span class="meta">Scout → API · clients.md @ a91f</span></span></button><button class="artifact" data-action="patch">${icon('file')}<span><b>API + UI patch bundle</b><br><span class="meta">2 revisions · base 8bf0 · candidate c7e1</span></span></button><button class="artifact" data-action="review">${icon('check')}<span><b>Reviewer’s report</b><br><span class="meta">Exact revisions reviewed · no open blocker</span></span></button><div class="section-label">Total observed spend <span>$0.68</span></div><div class="spend"><i style="width:20%;--role:var(--accent)"></i><i style="width:10%;--role:var(--read)"></i><i style="width:30%;--role:var(--edit)"></i><i style="width:28%;--role:var(--accent)"></i><i style="width:12%;--role:var(--green)"></i></div><p class="meta">Pi $0.14 · Scout $0.07 · API $0.20<br>UI $0.19 · Reviewer $0.08</p><div class="callout"><b>Main workspace unchanged by integration</b>Agents edited isolated worktrees. The candidate is not yet applied to the main project. Review is inspection, not Accept.</div>`;
}
function offlineView(s) {
  return `<span class="label">Connection lost / s-84</span><h2 style="margin-top:12px">The computer<br>is out of reach.</h2><p class="secondary">Last update from studio-mac at 09:42:18. Pi does not run on this phone.</p><div class="callout wait"><b>Current swarm state: unknown</b>We can show the last observation, not whether the agents are still working.</div><div class="section-label">Last seen <span>2 working · 2 waiting · 1 done</span></div>${graph(s,true)}<p class="meta" style="margin-top:16px">Unsaved phone actions were not sent. Reconnection replays committed events after cursor 13, then refreshes the projection.</p><p class="meta" style="margin-top:12px">Stop and Answer are unavailable until the computer acknowledges a connection.</p>`;
}
function sheetContent(s) {
  const close='<button class="icon-btn" data-action="close" aria-label="Close sheet">'+icon('close')+'</button>';
  let title='',body='',footer='';
  switch(s.sheet) {
    case 'exchange':
      title='API ↔ UI';
      body=`<p class="meta">Swarm s-84 · ordered peer exchange</p><div class="message-bubble"><div class="row between"><b>API → UI · Question #13</b>${badge('Delivered','wait')}</div><p>Can the client persist an opaque cursor? I need the response contract before I edit.</p><div class="receipts">09:42:15 accepted by broker<br>09:42:16 delivered to UI’s context<br>correlation: cursor-contract / expires 09:44:15</div></div>${s.replied?'<div class="message-bubble"><b>UI → API · Reply #14 → #13</b><p>Yes. Persist it per session; reset it when the snapshot version changes.</p><div class="receipts">09:42:24 accepted · 09:42:25 delivered<br>API’s wait resolved by reply #14</div></div>':closedExchange(s)?'<div class="callout wait"><b>Wait cancelled by peer Stop</b>Delivery #13 is historical. No reply can reopen this assignment.</div>':'<div class="callout wait"><b>No reply yet</b>UI is comparing cursor formats. A delivery receipt does not unblock API.</div>'}<p class="meta" style="margin-top:16px">These are explicit messages agents chose to share—not their hidden reasoning. Inspecting this sheet does not change work.</p>`;
      if(!s.replied&&s.connected&&!closedExchange(s)&&s.lifecycle==='active') footer='<button class="btn wide" data-action="sample-reply">Simulate UI reply · sample only</button>';
      break;
    case 'artifact':
      title='clients.md';
      body='<p class="meta">Published by Scout · artifact f-12 · revision a91f</p><div class="code"># Compatibility constraints\n\n1. Android v1 expects full snapshots.\n2. Reconnect must deduplicate prompts.\n3. Cursor state is per session.\n\nSource: src/protocol.ts + client call sites</div><div class="callout"><b>Immutable, read-only reference</b>A later edit creates a new revision. Recipients cannot change this artifact or gain access to arbitrary computer files.</div>';
      break;
    case 'assignment': title='Pi → Reviewer'; body='<div class="message-bubble"><b>Assignment #11 · delivered 09:41:52</b><p>Review both patch revisions, then check the integration candidate.</p></div><p class="meta">The reviewer is waiting for artifact readiness, not for more instructions. Its transcript remains separate.</p>'; break;
    case 'transcript':
      title=definitions[s.agent].name+'’s conversation';
      body=`<p class="meta">${s.connected?'Loaded on demand':'Cached before disconnect'} · sample content</p><div class="detail-brief">${definitions[s.agent].task}</div><div class="message-bubble"><b>Observed tool call</b><p>read · src/protocol.ts</p><div class="code">export interface SessionUpdate {\n  version: number;\n  entries: SessionEntry[];\n}</div></div><div class="message-bubble"><b>Observed activity</b><p>${s.agent==='api'?'Asked UI about cursor persistence. See the exchange for the current outcome.':'Worked on its delegated brief and published a bounded progress update.'}</p></div><p class="meta">No private reasoning is shown. The phone requests only this conversation, not every swarm transcript.</p>`;
      break;
    case 'steer':
      title='Steer Pi';
      body=`<label for="steer-draft">Message to the coordinator</label><textarea id="steer-draft" maxlength="2000" placeholder="Ask Pi to change direction…">${escapeHTML(s.draft)}</textarea><p class="meta">Pi decides what to relay. This does not broadcast your text to every agent or silently change tool permissions.</p>`;
      footer='<button class="btn primary wide" data-action="send-steer">Send to Pi · sample only</button>';
      break;
    case 'stop-agent':
      title='Stop '+definitions[s.agent].name+'?';
      body='<p>Abort this agent’s active turn and close its mailbox. Other agents keep working; Pi is told which dependent waits can no longer complete.</p><div class="callout"><b>Existing edits are kept</b>This does not undo file changes or guarantee a command’s external effects were rolled back.</div><p class="meta" style="margin-top:16px">The computer must acknowledge Stop. Until then, the agent is “Stopping”, not “Stopped”.</p>';
      footer='<button class="btn danger wide" data-action="confirm-stop-agent">Stop this agent · sample only</button>';
      break;
    case 'stop-all':
      title='Stop the whole swarm?';
      body='<p>Stop Pi and all four peers, reject new admissions and cancel undelivered messages. Waiting peers receive cancellation instead of waiting forever.</p><div class="callout"><b>Keep artifacts and project edits</b>Stop is not undo. Any interrupted command is reported, never automatically repeated.</div>';
      footer='<button class="btn danger wide" data-action="confirm-stop-all">Stop session & peers · sample only</button>';
      break;
    case 'group': {
      const g=groups.find(group=>group[0]===s.selectedGroup)||groups[0]; title=g[0]+' · '+g[1]+' agents';
      body=`<p class="meta">Every group member, not a hidden “+more” cluster.</p>${Array.from({length:g[1]},(_,i)=>`<div class="agent-row"><span class="tile" style="--role:var(--${g[5]})">${String(i+1).padStart(2,'0')}</span><span class="agent-text"><b>${g[0]}-${String(i+1).padStart(2,'0')}</b><span class="meta">${['Protocol shapes','Reconnect handling','Compatibility path','Projection fields','Serialization','Integration checks','Regression cases','Output fixtures'][i]}</span></span>${badge(i<g[2]?'Working':i<g[2]+g[3]?'Waiting':'Done',i<g[2]?'':i<g[2]+g[3]?'wait':'good')}</div>`).join('')}<div class="callout">The live product would open each member’s exact conversation and workspace. This larger fixture is a roster-only study.</div>`;
      break;
    }
    case 'find': title='Find a group'; body='<label for="agent-search">Name or responsibility</label><input id="agent-search" type="search" placeholder="Try API, UI or review" style="width:100%;min-height:48px;padding:12px;border:1px solid var(--strong-line);border-radius:12px;background:var(--paper);margin:8px 0"><div id="agent-search-results">'+groups.map(g=>`<button class="agent-row" data-group="${g[0]}"><b>${g[0]}</b><span class="spacer"></span><span class="meta">${g[1]} agents</span>${icon('chevron')}</button>`).join('')+'</div>'; break;
    case 'large-blocker': title='UI-04 waits on API-02'; body='<div class="message-bubble"><b>Artifact request #38 · delivered</b><p>Publish the schema revision so UI-04 can finalize its decoder.</p></div><div class="callout wait"><b>Affected peers</b>UI-04 and Tests-03. This is a dependency view, not proof of a deadlock.</div><p class="meta" style="margin-top:16px">API-02 is still working. If its deadline expires, Pi receives the blocker and can reassign or cancel the waiting work.</p>'; break;
    case 'patch': title='Integration candidate'; body='<p>Two patch artifacts, built from base 8bf0:</p><div class="code">API  patch p-18 @ 29ca\nUI   patch p-19 @ b40e\n\nPi integration candidate @ c7e1\n3 changed files · +84 / −12</div><div class="callout"><b>Separate worktrees</b>Pi serialized integration and Reviewer tested c7e1. Neither peer wrote into the other’s files.</div>'; break;
    case 'review': title='Reviewer’s report'; body='<div class="message-bubble"><b>Finding #21 · delivered to Pi</b><p>No open blockers in candidate c7e1. Compatibility cases passed; the UI decoder handles a reset cursor.</p></div><div class="code">bun test test/cursor.test.ts\n32 passed · 0 failed · exit 0\nrevision c7e1 · 09:47:28</div><p class="meta" style="margin-top:16px">Check results apply to this candidate and command only. They are not a guarantee that every behavior is correct.</p>'; break;
    case 'review-changes': title='Review candidate c7e1'; body='<p class="meta">Observed edits in the integration worktree. Nothing is applied by opening Review.</p><div class="code">src/protocol.ts  +42 −8\nsrc/projection.ts +28 −4\ntests/cursor.ts   +14\n\n+ cursor?: string\n+ resetCursor: boolean</div><div class="callout">The product would use Android’s existing one-file-at-a-time Review screen. Applying a candidate needs a separate, confirmed computer-side operation.</div>'; break;
    case 'about': title='Swarm scope'; body='<p>One session, one computer, separate agent conversations. All peer messages stay inside this swarm and pass through a durable broker.</p><div class="callout"><b>Proposed, not implemented</b>The existing runtime has subagent crews, chains and direct child steering. Peer messaging, broker receipts and this map are new work.</div>'; break;
  }
  return `<div class="scrim"><section class="sheet" role="dialog" aria-modal="true" aria-label="${escapeHTML(title)}"><div class="sheet-handle" aria-hidden="true"></div><div class="sheet-head"><h3>${escapeHTML(title)}</h3>${close}</div><div class="sheet-body">${body}</div>${footer}</section></div>`;
}
function dock(s,route) {
  const btn=(text,action,cls='')=>`<button class="btn ${cls}" data-action="${action}">${text}</button>`;
  if(!s.connected) return `<div class="dock"><p class="meta">No offline Stop, Answer or Send</p>${btn('Simulate reconnect · sample only','reconnect','primary wide')}</div>`;
  if(s.stopping) return `<div class="dock"><p class="meta">Stop requested; acknowledgement pending · sample only</p>${btn('Simulate Stop acknowledgement','ack-stop','wide')}</div>`;
  if(route==='question') return `<div class="dock"><p class="meta">${s.answer?'Sample decision recorded':'Only an explicit Answer sends your choice'}</p><div class="dock-row">${btn('Later','later')}${s.answer?btn('Back to swarm','back-map','primary grow'):`<button class="btn primary grow" data-action="answer" ${s.choice?'':'disabled'}>Answer · sample only</button>`}</div></div>`;
  if(route==='done'||s.lifecycle==='finished') return `<div class="dock"><p class="meta">Candidate c7e1 · main project not integrated</p>${btn('Review changes','review-changes','primary wide')}</div>`;
  if(route==='agent') return `<div class="dock"><p class="meta">Only this agent stops; peers carry on</p><div class="dock-row">${btn('Back to swarm','back-map','grow')}${agentStatus(s.agent,s)==='Done'?'<button class="btn" disabled>Done</button>':s.stopped.has(s.agent)||s.allStopped?'<button class="btn" disabled>Stopped</button>':btn('Stop '+definitions[s.agent].name,'stop-agent','danger')}</div></div>`;
  return `<div class="dock"><p class="meta">${route==='many'?'24 peers on studio-mac':'Work stays on studio-mac when the phone leaves'}</p><div class="dock-row">${s.allStopped?'<button class="btn grow" disabled>Session stopped</button>':btn('Steer Pi','steer','primary grow')}${s.allStopped?'<button class="btn" disabled>Stopped</button>':route==='many'?btn('Limits','limits'):btn(icon('stop')+' Stop','stop-all','danger')}</div></div>`;
}
function renderPhone(host,route,s,theme) {
  s={...s,uid:host.id};
  host.classList.toggle('evening',theme==='evening');
  host.dataset.screen=route;
  const title=route==='agent'?definitions[s.agent].name:route==='question'?'Pi needs your decision':'Swarm';
  const view={map:()=>mapView(s),messages:()=>messagesView(s),agents:()=>`<span class="label">Swarm / s-84</span><h2 style="margin-top:10px">Who’s doing what.</h2>${tabs('agents')}${Object.keys(definitions).map(key=>agentRow(key,s)).join('')}<div class="callout"><b>Independent conversations</b>Each peer sees its brief and scoped messages, not everyone’s transcript.</div>`,agent:()=>agentView(s),question:()=>questionView(s),many:()=>manyView(s),offline:()=>offlineView(s),done:()=>doneView(s)}[route]||(()=>mapView(s));
  const clock=s.scene==='finished'?'9:48':s.scene==='decision'?'9:43':'9:42';
  host.innerHTML=`<div class="status"><span>${clock}</span><span class="spacer"></span>${icon('wifi')}${icon('battery')}</div><header class="topbar"><button class="icon-btn" data-action="back" aria-label="Back to swarm">${icon('back')}</button><div class="title"><b>${title}</b><small>Session API update · studio-mac</small></div><button class="icon-btn" data-action="about" aria-label="About this swarm">${icon('more')}</button></header><main class="screen-scroll">${view()}</main>${dock(s,route)}<div class="gesture" aria-hidden="true"></div>${s.sheet?sheetContent(s):''}`;
}
function notify(text) { const el=document.getElementById('toast'); el.textContent=text; el.classList.add('visible'); clearTimeout(toastTimer); toastTimer=setTimeout(()=>el.classList.remove('visible'),4500); }
let returnFocus=null;
let inertElements=[];
function clearInert() { inertElements.forEach(el=>el.inert=false); inertElements=[]; }
function isolateDialog(dialog) {
  let branch=dialog;
  while(branch.parentElement && branch.parentElement!==document.documentElement) {
    for(const sibling of branch.parentElement.children) {
      if(sibling!==branch&&!sibling.inert&&sibling.id!=='toast'&&sibling.tagName!=='SCRIPT') { sibling.inert=true; inertElements.push(sibling); }
    }
    branch=branch.parentElement;
  }
}
function focusHeading() { const heading=demoHost().querySelector('main h2'); if(heading) { heading.tabIndex=-1; heading.focus({preventScroll:true}); } }
const singleParams=new URLSearchParams(location.search);
const isSingle=singleParams.get('single')==='1';
function demoHost() { return document.getElementById(isSingle?'single-phone':'demo-phone'); }
function refresh(focusSheet=false) {
  clearInert();
  renderPhone(demoHost(),state.route,state);
  if(!isSingle) document.getElementById('route').value=state.route==='agents'?'map':state.route;
  if(state.sheet) {
    isolateDialog(demoHost().querySelector('[role="dialog"]'));
    if(focusSheet) demoHost().querySelector('[data-action="close"]').focus();
  }
}
function openSheet(name) { returnFocus=document.activeElement; state.sheet=name; refresh(true); }
function closeSheet() { const data=returnFocus?.dataset; state.sheet=null; refresh(); if(data) { const key=['action','agent','group'].find(k=>data[k]); if(key) demoHost().querySelector(`[data-${key}="${data[key]}"]`)?.focus(); } }
function navigate(route) {
  if(!state.connected) route='offline';
  else if(state.scene==='many'&&route==='map') route='many';
  state.previous=state.route; state.route=route; state.sheet=null; refresh(); focusHeading();
}
function loadFixture(route) { clearInert(); state=fixture(route); refresh(); focusHeading(); }
function showDemo(route) {
  if(isSingle) { loadFixture(route); return; }
  document.getElementById('gallery').hidden=true;
  document.getElementById('demo').hidden=false;
  document.querySelectorAll('[data-view]').forEach(btn=>btn.setAttribute('aria-pressed',String(btn.dataset.view==='demo')));
  if(route) state=fixture(route);
  refresh();
}
function handleClick(event) {
  const button=event.target.closest('button'); if(!button||button.disabled) return;
  const host=button.closest('.phone');
  if(host && host!==demoHost()) { showDemo(host.dataset.screen); notify('Interactive sample opened. No work runs.'); return; }
  if(button.dataset.view) {
    const gallery=button.dataset.view==='gallery'; document.getElementById('gallery').hidden=!gallery; document.getElementById('demo').hidden=gallery; document.querySelectorAll('[data-view]').forEach(btn=>btn.setAttribute('aria-pressed',String(btn===button))); if(!gallery) refresh(); return;
  }
  if(button.dataset.try) { showDemo(button.dataset.try); return; }
  if(button.id==='theme-toggle') { const evening=document.documentElement.dataset.theme!=='evening'; document.documentElement.dataset.theme=evening?'evening':'moonstone'; button.setAttribute('aria-pressed',String(evening)); button.textContent=evening?'Moonstone theme':'Evening theme'; return; }
  if(button.id==='reset') { loadFixture('map'); notify('Sample reset.'); return; }
  if(button.dataset.route) { navigate(button.dataset.route); return; }
  if(button.dataset.agent) { state.agent=button.dataset.agent; if(!state.connected) { openSheet('transcript'); return; } navigate('agent'); return; }
  if(button.dataset.filter) { state.filter=button.dataset.filter; refresh(); demoHost().querySelector(`[data-filter="${state.filter}"]`).focus(); return; }
  if(button.dataset.choice) { state.choice=button.dataset.choice; refresh(); demoHost().querySelector(`[data-choice="${state.choice}"]`).focus(); return; }
  if(button.dataset.group) { state.selectedGroup=button.dataset.group; openSheet('group'); return; }
  const action=button.dataset.action;
  const mutations=['steer','stop-agent','stop-all','answer','sample-reply','confirm-stop-agent','confirm-stop-all','send-steer','ack-stop'];
  if(mutations.includes(action)&&(!state.connected||state.allStopped||state.lifecycle!=='active'||(state.stopping&&action!=='ack-stop'))) { notify('This sample is offline or the assignment has ended; no new action is admitted.'); return; }
  if(['exchange','artifact','transcript','steer','stop-agent','stop-all','about','assignment','find','large-blocker','patch','review','review-changes'].includes(action)) { openSheet(action); return; }
  switch(action) {
    case 'close': closeSheet(); break;
    case 'back': if(state.route==='map') notify('This study keeps you inside the sample session.'); else navigate('map'); break;
    case 'back-map': case 'later': navigate('map'); break;
    case 'answer': if(!state.choice)return; state.answer=state.choice; state.choice=null; refresh(); focusHeading(); notify('Sample answer acknowledged. No decision was sent to Pi.'); break;
    case 'sample-reply': if(closedExchange(state))return; state.replied=true; refresh(true); notify('Sample reply #14 delivered; API is no longer waiting.'); break;
    case 'confirm-stop-agent': state.stopping=state.agent; state.sheet=null; refresh(); demoHost().querySelector('[data-action="ack-stop"]').focus(); notify('Sample Stop requested. Still waiting for the computer’s acknowledgement.'); break;
    case 'confirm-stop-all': state.stopping='all'; state.sheet=null; navigate('map'); notify('Sample Stop requested, not yet acknowledged.'); break;
    case 'ack-stop': if(state.stopping==='all')state.allStopped=true; else if(state.stopping)state.stopped.add(state.stopping); state.stopping=null; refresh(); focusHeading(); notify('Sample Stop acknowledged. Edits are kept; completed work remains Done.'); break;
    case 'send-steer': if(!state.draft.trim()) { notify('Write a message to Pi first.'); return; } state.draft=''; closeSheet(); notify('Sample message admitted to Pi. No prompt was submitted.'); break;
    case 'reconnect': state.connected=true; state.scene='live'; navigate('map'); notify('Simulated replay after cursor 13. No network connection was made.'); break;
    case 'limits': notify('Sample limits: 8 concurrent peers plus a bounded coordinator lane, 24 peers, $3.00 admission cap. Limits are enforced on the computer in the proposal.'); break;
  }
}
document.addEventListener('click',handleClick);
document.addEventListener('input',event=> {
  if(event.target.id==='steer-draft') state.draft=event.target.value;
  if(event.target.id==='agent-search') {
    const query=event.target.value.trim().toLowerCase();
    const results=groups.filter(g=>g[0].toLowerCase().includes(query));
    document.getElementById('agent-search-results').innerHTML=results.length?results.map(g=>`<button class="agent-row" data-group="${g[0]}"><b>${g[0]}</b><span class="spacer"></span><span class="meta">${g[1]} agents</span>${icon('chevron')}</button>`).join(''):'<p class="meta">No matching agent group.</p>';
  }
});
document.addEventListener('keydown',event=> {
  if(event.key==='Escape') { if(state.sheet) closeSheet(); else if(state.route!=='map') navigate('map'); }
  if(event.key==='Tab'&&state.sheet) {
    const items=[...demoHost().querySelectorAll('.sheet button:not(:disabled), .sheet textarea, .sheet input, .sheet [tabindex="0"]')];
    const first=items[0],last=items[items.length-1];
    if(!demoHost().querySelector('.sheet').contains(document.activeElement)) { event.preventDefault(); first.focus(); }
    else if(event.shiftKey&&document.activeElement===first) { event.preventDefault(); last.focus(); }
    else if(!event.shiftKey&&document.activeElement===last) { event.preventDefault(); first.focus(); }
  }
});
if(isSingle) {
  document.body.classList.add('single'); document.getElementById('study').hidden=true; const single=document.getElementById('single'); single.hidden=false; single.innerHTML='<div class="phone" id="single-phone"></div>';
  state=fixture(singleParams.get('screen')||'map'); if(singleParams.get('theme')==='evening') document.documentElement.dataset.theme='evening'; refresh();
} else {
  if(matchMedia('(prefers-color-scheme: dark)').matches) { document.documentElement.dataset.theme='evening'; document.getElementById('theme-toggle').textContent='Moonstone theme'; document.getElementById('theme-toggle').setAttribute('aria-pressed','true'); }
  const gallery=document.getElementById('gallery');
  views.forEach(([file,route,title,note,theme],index)=> {
    const figure=document.createElement('figure'); figure.innerHTML=`<div class="figure-head"><span>${String(index+1).padStart(2,'0')}</span><h2>${title}</h2></div><div class="phone" id="gallery-phone-${index}"></div><figcaption>${note}<button data-try="${route}">Try this screen ↗</button></figcaption>`; gallery.append(figure);
    renderPhone(figure.querySelector('.phone'),route,fixture(route),theme);
  });
  document.getElementById('route').addEventListener('change',event=>loadFixture(event.target.value)); refresh();
}
