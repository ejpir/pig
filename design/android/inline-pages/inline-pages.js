(() => {
  'use strict';
  const $ = id => document.getElementById(id);
  const frame = $('calculator-frame');
  const params = new URLSearchParams(location.search);
  const scenes = ['inline', 'update', 'offline', 'unavailable'];
  const state = { scene: 'inline', version: 1, expanded: false, saved: false, modal: null, result: '192', expression: '128 + 64 =', source: '', draft: '', returnFocus: null, toastTimer: null };
  const paths = {
    check: '<path d="m5 12 4 4L19 6"/>',
    lock: '<rect x="5" y="10" width="14" height="11" rx="3"/><path d="M8 10V7a4 4 0 0 1 8 0v3"/>',
    code: '<path d="m8 7-5 5 5 5m8-10 5 5-5 5m-3-14-2 18"/>',
    cloud: '<path d="M5 17a4 4 0 0 1-1-8 7 7 0 0 1 13-1 5 5 0 0 1 2 9M4 4l16 16"/>',
  };
  const icon = name => `<svg viewBox="0 0 24 24" aria-hidden="true">${paths[name] || paths.check}</svg>`;
  function toast(message) {
    clearTimeout(state.toastTimer);
    $('toast').textContent = message;
    $('toast').classList.add('visible');
    state.toastTimer = setTimeout(() => $('toast').classList.remove('visible'), 3500);
  }

  // A real, isolated HTML artifact. No eval, app bridge, storage, or network.
  // The same iframe stays mounted when the chat changes or the page expands.
  function calculatorHTML(version) {
    const memory = version === 2;
    const css = `
:root{--paper:#fff;--canvas:#f6f5f2;--key:#f2f0ec;--key-hover:#e8e4de;--text:#253140;--muted:#647080;--display:#edf0f3;--operator:#e4eaf0;--accent:#4b607c;--on-accent:#fff;--line:#ded8d1}
:root[data-theme="evening"]{--paper:#1e2834;--canvas:#171e28;--key:#293441;--key-hover:#344252;--text:#ebe7e4;--muted:#a4b1c1;--display:#17212d;--operator:#334556;--accent:#9bb7d2;--on-accent:#172434;--line:#35404d}
*{box-sizing:border-box}html,body{height:100%}body{margin:0;background:var(--paper);color:var(--text);font:14px/1.4 system-ui,sans-serif}button{font:inherit;color:inherit;cursor:pointer;-webkit-tap-highlight-color:transparent;touch-action:manipulation}button:focus-visible{outline:3px solid var(--accent);outline-offset:-4px}.calculator{padding:14px;height:100%;min-height:${memory ? 432 : 376}px;display:flex;flex-direction:column;gap:8px}.display{background:var(--display);padding:10px 16px;border-radius:12px;min-height:74px;display:flex;flex-direction:column;justify-content:center;align-items:flex-end;overflow:hidden;flex:1}.expression{font-size:11px;color:var(--muted);height:18px;white-space:nowrap;max-width:100%;overflow:hidden;text-overflow:ellipsis}.result{font-size:36px;line-height:43px;letter-spacing:-.045em;font-weight:400;font-variant-numeric:tabular-nums;max-width:100%;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}.result.error{font-size:19px;letter-spacing:-.02em}.keys{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));grid-template-rows:repeat(5,minmax(48px,1fr));gap:6px;flex:3}.keys button{min-height:48px;border:0;border-radius:10px;background:var(--key);font-size:20px;font-weight:400;padding:0}.keys button:hover{background:var(--key-hover)}.keys .function{font-size:13px;color:var(--muted)}.keys .operator{background:var(--operator);color:var(--accent);font-size:23px}.keys .equals{background:var(--accent);color:var(--on-accent)}.keys .zero{grid-column:span 2}.memory{display:flex;gap:6px;flex:none}.memory button{flex:1;min-height:48px;border:1px solid var(--line);border-radius:10px;background:var(--paper);font-size:12px;font-weight:600}.memory-mark{float:left;font-size:10px;margin-right:8px;color:var(--muted)}@media(prefers-reduced-motion:reduce){*{transition:none!important}}`;
    const calcJS = `
(() => {
  'use strict';
  let value = '${memory ? '0' : '192'}', expression = '${memory ? '' : '128 + 64 ='}', stored = null, operator = null, fresh = true, repeat = null, memory = 0;
  const result = document.getElementById('result');
  const expressionEl = document.getElementById('expression');
  const symbols = {'+':'+','-':'−','*':'×','/':'÷'};
  const format = number => Number.isFinite(number) ? String(Number(number.toPrecision(12))) : 'Error';
  function publish() {
    result.textContent = value;
    result.classList.toggle('error', value === 'Can’t divide by zero' || value === 'Error');
    expressionEl.textContent = expression || ' ';
    const mark = document.getElementById('memory-mark');
    if (mark) mark.textContent = memory ? 'M' : '';
    parent.postMessage({type:'calculator-state', value, expression}, '*');
  }
  function calculate(a, b, op) {
    if (op === '/' && b === 0) return 'Can’t divide by zero';
    return format(op === '+' ? a+b : op === '-' ? a-b : op === '*' ? a*b : a/b);
  }
  function clear() { value='0'; expression=''; stored=null; operator=null; fresh=true; repeat=null; }
  function press(key) {
    if (/^[0-9]$/.test(key)) {
      if (fresh || !Number.isFinite(Number(value))) { value=key; fresh=false; }
      else if (value.replace(/[^0-9]/g,'').length < 12) value=value==='0'?key:value+key;
      if (!operator) expression='';
      repeat=null;
    } else if (key === '.') {
      if (fresh || !Number.isFinite(Number(value))) { value='0.'; fresh=false; }
      else if (!value.includes('.')) value+='.';
      if (!operator) expression='';
      repeat=null;
    } else if (key === 'AC') clear();
    else if (key === 'Backspace') { value=fresh?'0':value.slice(0,-1)||'0'; repeat=null; }
    else if (key === 'sign' && Number.isFinite(Number(value))) { value=format(-Number(value)); repeat=null; }
    else if (key === '%' && Number.isFinite(Number(value))) { value=format(Number(value)/100); repeat=null; }
    else if (key === 'M+' && Number.isFinite(Number(value))) { memory+=Number(value); fresh=true; }
    else if (key === 'MR') { value=format(memory); fresh=true; }
    else if (key === 'MC') memory=0;
    else if (['+','-','*','/'].includes(key) && Number.isFinite(Number(value))) {
      if (operator && !fresh) value=calculate(stored,Number(value),operator);
      if (!Number.isFinite(Number(value))) { stored=null; operator=null; fresh=true; publish(); return; }
      stored=Number(value); operator=key; expression=value+' '+symbols[key]; fresh=true; repeat=null;
    } else if (key === '=' && Number.isFinite(Number(value))) {
      if (operator) {
        const right=Number(value); expression=stored+' '+symbols[operator]+' '+value+' =';
        value=calculate(stored,right,operator); repeat={op:operator,right}; stored=null; operator=null; fresh=true;
      } else if (repeat) {
        const left=Number(value); expression=value+' '+symbols[repeat.op]+' '+repeat.right+' =';
        value=calculate(left,repeat.right,repeat.op); fresh=true;
      }
    }
    publish();
  }
  document.addEventListener('click', event => { const button=event.target.closest('[data-key]'); if (button) press(button.dataset.key); });
  document.addEventListener('keydown', event => {
    // Leave Enter/Space on a focused key to native button activation.
    if ((event.key==='Enter'||event.key===' ') && event.target.matches('button')) return;
    const key=event.key==='Enter'?'=':event.key==='Escape'?'AC':event.key;
    if (/^[0-9.+*/%=-]$/.test(key)||key==='AC'||key==='Backspace') { event.preventDefault(); press(key); }
  });
  window.addEventListener('message', event => {
    if (event.source!==parent || !event.data || event.data.type!=='calculator-theme') return;
    document.documentElement.dataset.theme=event.data.theme==='evening'?'evening':'moonstone';
  });
  publish();
})();`;
    const keys = [['AC','AC','function'],['sign','+/−','function'],['%','%','function'],['/','÷','operator'],['7','7',''],['8','8',''],['9','9',''],['*','×','operator'],['4','4',''],['5','5',''],['6','6',''],['-','−','operator'],['1','1',''],['2','2',''],['3','3',''],['+','+','operator'],['0','0','zero'],['.','.',''],['=','=','equals']];
    const names = {AC:'Clear all',sign:'Change sign','%':'Percent','/':'Divide','*':'Multiply','-':'Subtract','+':'Add','=':'Equals','.':'Decimal point'};
    return `<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data:; font-src data:; connect-src 'none'; form-action 'none'; base-uri 'none'">
<title>Calculator</title><style>${css}</style></head>
<body><main class="calculator" aria-label="Calculator">
<div class="display" tabindex="0" aria-label="Calculator display; type numbers and press Enter to calculate"><div class="expression" id="expression">${memory ? '' : '128 + 64 ='}</div><div class="result" id="result" role="status" aria-live="polite">${memory ? '0' : '192'}</div></div>
${memory ? '<div class="memory"><button data-key="MC" aria-label="Clear memory">MC</button><button data-key="MR" aria-label="Recall memory"><span id="memory-mark" class="memory-mark"></span>MR</button><button data-key="M+" aria-label="Add to memory">M+</button></div>' : ''}
<div class="keys">${keys.map(([key,label,kind]) => `<button class="${kind}" data-key="${key}" aria-label="${names[key] || label}">${label}</button>`).join('\n')}</div>
</main><script>${calcJS}${'</' + 'script>'}</body></html>`;
  }
  function themeMessage() { frame.contentWindow?.postMessage({ type:'calculator-theme', theme:document.documentElement.dataset.theme }, '*'); }
  frame.addEventListener('load', themeMessage);
  window.addEventListener('message', event => {
    if (event.source !== frame.contentWindow || !event.data || event.data.type !== 'calculator-state') return;
    if (typeof event.data.value !== 'string' || typeof event.data.expression !== 'string') return;
    if (event.data.value.length > 100 || event.data.expression.length > 150) return;
    state.result = event.data.value;
    state.expression = event.data.expression;
  });
  function mountVersion(version) {
    state.version = version;
    state.source = calculatorHTML(version);
    state.result = version === 1 ? '192' : '0';
    state.expression = version === 1 ? '128 + 64 =' : '';
    frame.srcdoc = state.source;
    $('widget-surface').classList.toggle('memory', version === 2);
    $('artifact-file').textContent = `calculator.html · v${version}`;
  }

  function setTheme(theme) {
    document.documentElement.dataset.theme = theme === 'evening' ? 'evening' : 'moonstone';
    $('theme-toggle').setAttribute('aria-pressed', String(theme === 'evening'));
    $('theme-toggle').textContent = theme === 'evening' ? 'Moonstone theme' : 'Evening theme';
    themeMessage();
  }
  function syncDraft() {
    state.draft = $('draft').value;
    const offline = state.scene === 'offline';
    $('send-button').disabled = offline || !state.draft.trim();
    $('draft').placeholder = offline ? 'Draft a change for later…' : 'Ask for a change…';
    $('composer-hint').textContent = offline ? 'Offline · draft stays here' : 'Claude Sonnet · Medium ⌄';
  }
  $('draft').addEventListener('input', syncDraft);
  function setExpanded(expanded) {
    state.expanded = expanded;
    $('phone').classList.toggle('expanded', expanded);
    $('expanded-context').hidden = !expanded;
    $('session-title').textContent = expanded ? 'Calculator' : 'Build a calculator';
    $('expand-button').setAttribute('aria-label', expanded ? 'Return calculator to chat' : 'Expand calculator');
    $('expand-button').title = expanded ? 'Return to chat' : 'Expand calculator';
    $('expand-button').innerHTML = expanded ? '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M9 4v5H4m11-5v5h5M4 15h5v5m6 0v-5h5M9 9 4 4m11 5 5-5M9 15l-5 5m11-5 5 5"/></svg>' : '<svg viewBox="0 0 24 24" aria-hidden="true"><path d="M8 4H4v4m12-4h4v4M4 16v4h4m12-4v4h-4M4 4l5 5m11-5-5 5M4 20l5-5m11 5-5-5"/></svg>';
    document.querySelector('[data-action="back"]').setAttribute('aria-label', expanded ? 'Return to chat' : 'Session navigation');
  }
  const descriptions = {
    inline: ['01 / In the conversation','Try 12 × 8.','Use AC to start over. The page also accepts keyboard input when focused.'],
    update: ['02 / A revision is ready','Your calculation comes first.','Pi can change the file without silently replacing the page you’re using. Applying v2 explicitly resets it.'],
    offline: ['03 / The computer is away','This part is already here.','HTML and local JavaScript stay on the phone. New prompts cannot be sent while the computer is disconnected.'],
    unavailable: ['04 / An edit we can’t follow','Unknown is not “unchanged.”','If a revision can’t be reconstructed, show that honestly. A saved version is available only by explicit choice.'],
  };
  function setScene(scene) {
    if (!scenes.includes(scene)) scene = 'inline';
    if (state.modal) closeSheet();
    setExpanded(false);
    state.scene = scene;
    state.saved = false;
    document.querySelectorAll('[data-scene]').forEach(button => {
      const selected = button.dataset.scene === scene;
      button.classList.toggle('selected', selected);
      button.setAttribute('aria-pressed', String(selected));
    });
    const [label, title, detail] = descriptions[scene];
    $('state-label').textContent = label;
    $('state-detail').replaceChildren();
    const strong = document.createElement('b'); strong.textContent = title;
    const p = document.createElement('p'); p.textContent = detail;
    $('state-detail').append(strong, p);
    const offline = scene === 'offline', unavailable = scene === 'unavailable';
    $('connection-banner').hidden = !offline;
    $('connection-dot').classList.toggle('offline', offline);
    $('host-label').textContent = offline ? 'studio-mac · disconnected' : 'studio-mac · tools';
    $('revision-notice').hidden = scene !== 'update' || state.version === 2;
    $('widget-surface').hidden = unavailable;
    $('unavailable-page').hidden = !unavailable;
    $('saved-notice').hidden = true;
    $('expand-button').disabled = unavailable;
    $('artifact-file').textContent = unavailable ? 'calculator.html · revision unavailable' : `calculator.html · v${state.version}`;
    $('page-status').textContent = unavailable ? 'No current preview' : 'On this phone · no network';
    document.querySelector('[data-action="source"]').disabled = unavailable;
    const copy = {
      inline: ['Build me a simple calculator I can use here.','Here you go. A small calculator, right in the conversation.','Want memory buttons or a different layout? Just ask.'],
      update: ['Add memory buttons to the calculator.','The new version is ready. Your current calculation is still here.','Update when you’re ready. Your open page won’t change on its own.'],
      offline: ['Build me a simple calculator I can use here.','Here you go. A small calculator, right in the conversation.','This saved page works without the computer. Pi will need a connection for changes.'],
      unavailable: ['Make the calculator a little more compact.','The file changed, but I can’t reconstruct this revision for the preview.','The file on the computer may be fine. This preview just doesn’t have its full contents.'],
    }[scene];
    if (scene === 'update' && state.version === 2) { copy[1] = 'Memory buttons are now available in this version.'; copy[2] = 'v2 is running. Your calculator is ready for a new calculation.'; }
    $('user-prompt').textContent = copy[0]; $('assistant-intro').textContent = copy[1]; $('after-page').textContent = copy[2];
    syncDraft();
    $('thread').scrollTop = 0;
  }

  function openSheet(kind, trigger) {
    if (state.modal) closeSheet(false);
    state.returnFocus = trigger instanceof HTMLElement ? trigger : document.activeElement;
    state.modal = kind;
    const content = $('sheet-content');
    const titles = {source:'Page source',permissions:'A page, with boundaries',update:'Update the running page?',session:'About this sample',attachments:'Attachments'};
    $('sheet-title').textContent = titles[kind] || 'About this sample';
    if (kind === 'source') {
      content.innerHTML = '<div class="source-topline"><span id="source-filename"></span><button data-action="copy-source">Copy source</button></div><pre class="source-code" id="source-code" tabindex="0" aria-label="Selectable HTML source"></pre><p class="sheet-caption">This is the HTML running in the frame. Viewing its source doesn’t reload the page.</p>';
      $('source-filename').textContent = `calculator.html · v${state.version}${state.saved ? ' · saved snapshot' : ''}`;
      $('source-code').textContent = state.source;
    } else if (kind === 'permissions') {
      content.innerHTML = `<p>Calculator runs locally, inside its own sandbox. It doesn’t inherit Pi’s tools or your computer’s permissions.</p><div class="permission-row">${icon('code')}<div><b>Local JavaScript</b><span>Buttons and calculations work on this phone, even without the computer.</span></div></div><div class="permission-row restricted">${icon('cloud')}<div><b>No network access</b><span>This self-contained page cannot fetch data, load remote assets or send your inputs to a server.</span></div></div><div class="permission-row restricted">${icon('lock')}<div><b>No app or computer access</b><span>No credentials, app storage, filesystem or native bridge. A page can’t send prompts or run commands.</span></div></div><p class="sheet-caption">Proposed native policy. This web study enforces a scripts-only iframe sandbox and blocks resource requests with a content security policy.</p><button class="button primary" data-action="close-sheet" style="width:100%">Got it</button>`;
    } else if (kind === 'update') {
      content.innerHTML = '<p>Pi added memory buttons. Updating replaces the running document; your current calculation won’t carry over.</p><div class="revision-comparison"><div><small>Running now · v1</small><b id="running-result"></b><span>Basic calculator</span></div><div><small>Ready to load · v2</small><b>MC · MR · M+</b><span>Starts a new calculation</span></div></div><p class="update-warning">Only this page resets. The conversation and your draft stay where they are.</p><div class="button-row"><button class="button" data-action="close-sheet">Keep v1</button><button class="button primary" data-action="apply-update">Load v2</button></div>';
      $('running-result').textContent = state.result;
    } else if (kind === 'attachments') {
      content.innerHTML = '<p>Attachments remain a native composer control, outside the page. No file picker is opened by this design sample.</p><button class="button primary" data-action="close-sheet" style="width:100%">Back to the conversation</button>';
    } else {
      content.innerHTML = '<p><b>Build a calculator</b><br>tools · studio-mac</p><p>This is an interactive proposal for Pi for Android. The calculator is real local HTML; the session, revisions and connection states are sample content.</p><p>The native app currently opens HTML pages separately. Inline native WebView hosting is not implemented by this study.</p><button class="button primary" data-action="close-sheet" style="width:100%">Back to the conversation</button>';
    }
    $('overlay').hidden = false;
    $('phone-main').inert = true;
    for (const id of ['study-controls','masthead','margin-notes','contracts','study-footer']) $(id).inert = true;
    $('sheet').querySelector('[data-action="close-sheet"]').focus();
  }
  function closeSheet(restoreFocus = true) {
    $('overlay').hidden = true;
    $('phone-main').inert = false;
    for (const id of ['study-controls','masthead','margin-notes','contracts','study-footer']) $(id).inert = false;
    state.modal = null;
    if (restoreFocus && state.returnFocus?.isConnected) state.returnFocus.focus({preventScroll:true});
  }
  async function copySource() {
    let copied = false;
    try { await navigator.clipboard.writeText(state.source); copied = true; } catch {}
    if (!copied) {
      const textarea = document.createElement('textarea');
      textarea.value = state.source; textarea.style.position='absolute'; textarea.style.opacity='0';
      $('sheet-content').append(textarea); textarea.select();
      try { copied = document.execCommand('copy'); } catch {}
      textarea.remove();
    }
    toast(copied ? 'HTML source copied.' : 'Select the source text to copy it.');
  }
  function savedPage() {
    // The fixture keeps v1 as a separately labeled snapshot, never an implicit fallback.
    if (state.version !== 1) mountVersion(1);
    state.saved = true;
    $('widget-surface').hidden = false;
    $('unavailable-page').hidden = true;
    $('saved-notice').hidden = false;
    $('artifact-file').textContent = 'calculator.html · saved v1';
    $('page-status').textContent = 'Saved snapshot · no network';
    $('expand-button').disabled = false;
    document.querySelector('[data-action="source"]').disabled = false;
    $('after-page').textContent = 'You’re using a saved v1 snapshot. The current file is still unavailable in this preview.';
  }
  function resetSample() {
    if (state.modal) closeSheet();
    mountVersion(1);
    $('draft').value = '';
    $('sample-followup').hidden = true;
    setScene('inline');
    toast('Sample reset. No session or files were changed.');
  }
  function sendSample() {
    if (state.scene === 'offline' || !$('draft').value.trim()) return;
    const draft = $('draft').value.trim();
    $('sample-followup-text').textContent = draft;
    $('sample-followup').hidden = false;
    $('draft').value = '';
    setScene('update');
    toast('Sample follow-up only. No prompt was sent to Pi.');
  }
  function action(name, button) {
    switch (name) {
      case 'expand': if ($('expand-button').disabled) return; setExpanded(!state.expanded); break;
      case 'back': if (state.expanded) setExpanded(false); else openSheet('session',button); break;
      case 'source': openSheet('source',button); break;
      case 'sandbox-info': openSheet('permissions',button); break;
      case 'review-update': openSheet('update',button); break;
      case 'close-sheet': closeSheet(); break;
      case 'copy-source': void copySource(); break;
      case 'apply-update': closeSheet(); mountVersion(2); setScene('update'); toast('v2 loaded. A new calculation is ready.'); break;
      case 'reconnect': setScene('inline'); toast('Sample connection restored. No SSH connection was made.'); break;
      case 'saved-page': savedPage(); break;
      case 'request-rewrite': $('draft').value='Rewrite calculator.html as a complete, self-contained HTML file so I can use it here.'; syncDraft(); $('draft').focus(); toast('Request added to your draft, not sent.'); break;
      case 'send': sendSample(); break;
      case 'session-info': openSheet('session',button); break;
      case 'attachments': openSheet('attachments',button); break;
    }
  }
  document.addEventListener('click', event => {
    const button = event.target.closest('button');
    if (!button || button.disabled) return;
    if (button.dataset.scene) setScene(button.dataset.scene);
    else if (button.dataset.action) action(button.dataset.action,button);
  });
  document.addEventListener('keydown', event => {
    if (event.key === 'Escape') {
      if (state.modal) { event.preventDefault(); closeSheet(); }
      else if (state.expanded) { event.preventDefault(); setExpanded(false); $('expand-button').focus({preventScroll:true}); }
    }
    if (event.key !== 'Tab' || !state.modal) return;
    const focusable = [...$('sheet').querySelectorAll('button:not(:disabled),a[href],textarea,[tabindex="0"]')];
    const first = focusable[0], last = focusable.at(-1);
    if (event.shiftKey && (document.activeElement === first || document.activeElement === $('sheet'))) { event.preventDefault(); last?.focus(); }
    else if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
  });
  $('reset-sample').addEventListener('click', resetSample);
  $('theme-toggle').addEventListener('click', () => setTheme(document.documentElement.dataset.theme === 'evening' ? 'moonstone' : 'evening'));
  if (params.get('single') === '1') document.body.classList.add('single');
  const requestedTheme = params.get('theme');
  setTheme(requestedTheme || (matchMedia('(prefers-color-scheme: dark)').matches ? 'evening' : 'moonstone'));
  mountVersion(1);
  setScene(params.get('scene') || 'inline');
  if (params.get('expanded') === '1') setExpanded(true);
  if (params.get('sheet') === 'source') openSheet('source', $('phone').querySelector('[data-action="source"]'));
})();
