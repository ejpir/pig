// Shared pieces for next5. Load before ../../next4/next4.js, which draws the
// icons and the status bar; ../icons.js comes after it.
//   <main class="content" data-v5="home|caught|working|done"> fills a screen

// The run line, from what happened in order: [kind, seconds, end]. A kind is
// look, change, check or wait; an end is pass or fail after a check and ask
// after a wait. `total` seconds fill `width` px. A track draws the rest of its
// scale behind, so on Home a run that has gone on longer shows longer.
const RUN = (steps, { width, total, small = false, live = false, track = false }) => {
  const gap = 2, px = width / total, size = small ? 10 : 14;
  const glyph = { pass: "check", fail: "x", ask: "hand" };
  let x = 0, bars = "", marks = "";
  steps.forEach(([kind, secs, end], n) => {
    const w = Math.max(small ? 3 : 4, Math.round(secs * px) - gap);
    bars += `<b class="${kind}${live && n === steps.length - 1 ? " live" : ""}" style="left:${x}px;width:${w}px"></b>`;
    if (end) marks += `<span class="mk ${end}" style="left:${Math.round(x + w - size / 2)}px"><i data-i="${glyph[end]}"></i></span>`;
    x += w + gap;
  });
  const rest = track ? `<b class="rest" style="left:0;width:${width}px"></b>` : "";
  return `<div class="rl${small ? " sm" : ""}" style="width:${width}px">${rest}${bars}${marks}</div>`;
};

// The same run as Android 16's ProgressStyle notification draws it: coloured
// segments, a point at each result, and a tracker where the run is now. The
// notification's own colours are fixed, so these are Moonstone's.
const PS = (steps, { width, total }) => {
  const hue = { look: "#2f7fa8", change: "#b0661a", check: "#949ca6" }, point = { pass: "#2e7950", fail: "#a44835" };
  const gap = 2, px = width / total;
  let x = 0, bars = `<b style="left:0;width:${width}px;background:#ddd7d0"></b>`, marks = "";
  steps.forEach(([kind, secs, end]) => {
    const w = Math.round(secs * px) - gap;
    bars += `<b style="left:${x}px;width:${w}px;background:${hue[kind]}"></b>`;
    if (end) marks += `<span class="pt" style="left:${x + w - 5}px;background:${point[end]}"></span>`;
    x += w + gap;
  });
  return `<div class="ps" style="width:${width}px">${bars}${marks}<span class="tracker" style="left:${x - 13}px"><i data-i="pi"></i></span></div>`;
};

// The sample runs, in seconds. Qwen signatures: a type check failed, Pi fixed
// it, the tests passed, then it wrote its hand-off (4:31).
const QWEN = [["look", 40], ["change", 70], ["check", 14, "fail"], ["change", 25], ["check", 92, "pass"], ["look", 30]];
// The same run at 2:51, its tests still running.
const QWEN_NOW = [["look", 40], ["change", 70], ["check", 14, "fail"], ["change", 25], ["check", 22]];
const KIMI = [["look", 30], ["change", 45], ["check", 20, "fail"], ["change", 25], ["check", 72]];
const KIMI_LATE = [["look", 30], ["change", 45], ["check", 20, "fail"], ["change", 25], ["check", 92, "pass"], ["look", 33]];
const RETRY_ANSWERED = [["look", 25], ["change", 60], ["wait", 70, "ask"], ["change", 35]];
const DOCS = [["look", 38]];

const BAR5 = (title, sub) => `
    <div class="bar">
      <span class="tap"><i data-i="back"></i></span>
      <div class="title"><b>${title}<i data-i="chev-d"></i></b><span>${sub}</span></div>
      <span class="tap"><i data-i="dots"></i></span>
    </div>`;

const SLIM = (placeholder) => `
      <div class="composer slim">
        <span class="tap"><i data-i="clip"></i></span><span class="draft ph">${placeholder}</span><span class="send off"><i data-i="send"></i></span>
      </div>`;

const HOME_BAR = `
    <div class="bar" style="padding-left:12px">
      <div class="row g12 grow">
        <span class="tile" style="--hue:var(--accent);width:40px;height:40px;border-radius:12px"><i data-i="pi"></i></span>
        <div class="grow">
          <div class="row g4"><b class="t3">studio-mac</b><i data-i="chev-d" class="sm muted"></i></div>
          <div class="row g8 meta"><span class="dot" style="--hue:var(--green);width:6px;height:6px"></span>Connected</div>
        </div>
      </div>
      <span class="tap"><i data-i="search"></i></span>
      <span class="tap"><i data-i="settings"></i></span>
    </div>`;

const START = `
    <div class="dock">
      <div class="composer row" style="height:56px;padding:0 8px 0 20px;border-radius:28px">
        <span class="grow faint">What should we change?</span>
        <span class="chip sm" style="margin-right:4px"><i data-i="folder"></i>pi</span>
        <span class="send"><i data-i="plus"></i></span>
      </div>
    </div>`;

// A working session on Home: its run on the shared five-minute track.
const WORKING = (name, time, steps, meta) => `
        <div class="item">
          <span class="lead"><span class="dot ring" style="--hue:var(--accent)"></span></span>
          <div class="grow"><div class="row"><span class="t ell grow">${name}</span><span class="meta">${time}</span></div>
            <div style="margin-top:8px">${RUN(steps, { width: 328, total: 300, small: true, live: true, track: true })}</div>
            <div class="meta ell" style="margin-top:6px">${meta}</div></div>
        </div>`;

const V5 = {
  // Home as an inbox: what needs you first, with the questions answerable in
  // place; then what is working; what you have seen folds away.
  home: `${HOME_BAR}
    <div class="scroll col" style="padding-top:4px">
      <div class="label" style="margin:8px 0">Needs you<span class="n">4</span></div>
      <div class="card" style="padding:16px;border-color:color-mix(in srgb,var(--needs) 45%,var(--line))">
        <div class="row g12">
          <span class="tile" style="--hue:var(--needs)"><i data-i="hand"></i></span>
          <div class="grow"><div class="strong ell">Gutter blame width</div><div class="meta">zed · asked 2 min ago · Pi is paused</div></div>
        </div>
        <div style="margin-top:12px;line-height:22px;font-weight:600">Clear the incremental build cache?</div>
        <div class="secondary" style="margin-top:2px;font-size:14px;line-height:20px">Stale entries are why the blame tests disagree.</div>
        <div class="well code" style="margin-top:8px;padding:9px 12px">rm -rf target/debug/incremental</div>
        <div class="row g8" style="margin-top:12px"><span class="btn sm grow">No</span><span class="btn sm primary grow">Yes</span></div>
      </div>
      <div class="list" style="margin-top:4px">
        <div class="item">
          <span class="lead"><span class="tile sm" style="--hue:var(--needs)"><i data-i="hand"></i></span></span>
          <div class="grow"><div class="t ell">Streaming retry</div><div class="meta ell" style="margin-top:2px">pi · asks where the jitter should go · 4 choices</div></div>
          <span class="side"><span class="new-dot"></span><span class="meta">now</span></span>
        </div>
        <div class="item">
          <span class="lead"><span class="tile sm" style="--hue:var(--accent)"><i data-i="diff"></i></span></span>
          <div class="grow"><div class="t ell">Qwen signatures</div><div class="meta ell" style="margin-top:2px">pi · ready to review · 2 files · checks passed</div></div>
          <span class="side"><span class="new-dot"></span><span class="meta">09:46</span></span>
        </div>
        <div class="item">
          <span class="lead"><span class="tile sm" style="--hue:var(--fail)"><i data-i="alert"></i></span></span>
          <div class="grow"><div class="t ell">VM snapshot restore</div><div class="meta ell" style="margin-top:2px">minivm · stopped: provider rate limit</div></div>
          <span class="side"><span class="new-dot"></span><span class="meta">09:15</span></span>
        </div>
      </div>
      <div class="label" style="margin:16px 0 0">Working<span class="n">2</span></div>
      <div class="col">${WORKING("Kimi K3 default", "3:12", KIMI, "pi · checking again · a type check failed once")}${WORKING("Pairing docs", "0:38", DOCS, "pi · looking · read 4 files")}
      </div>
      <div class="hair"></div>
      <div class="item one" style="color:var(--accent)"><span class="lead"><i data-i="clock" class="sm"></i></span><span class="grow strong" style="font-size:14px">Seen today</span><span class="meta">3</span><i data-i="chev-r" class="sm"></i></div>
    </div>${START}`,

  // The same Home once everything has been answered and reviewed.
  caught: `${HOME_BAR}
    <div class="scroll col" style="padding-top:4px">
      <div class="row g12" style="padding:16px 0 12px;align-items:flex-start">
        <span class="tile" style="--hue:var(--accent);width:40px;height:40px;border-radius:12px"><i data-i="inbox"></i></span>
        <div class="grow"><p class="headline">Nothing needs you.</p>
          <p class="meta" style="margin-top:4px;font-size:13px;line-height:18px">You'll hear when a run asks something, finishes or stops.</p></div>
      </div>
      <div class="label" style="margin:16px 0 0">Working<span class="n">2</span></div>
      <div class="col">${WORKING("Streaming retry", "3:10", RETRY_ANSWERED, "pi · changing · you answered “Only for OpenCode”")}${WORKING("Kimi K3 default", "4:05", KIMI_LATE, "pi · writing its hand-off · checks passed")}
      </div>
      <div class="label" style="margin:16px 0 0">Today</div>
      <div class="list">
        <div class="item seen"><span class="lead"><i data-i="check" class="sm muted"></i></span>
          <div class="grow"><div class="t ell">Qwen signatures</div><div class="meta ell" style="margin-top:2px">pi · reviewed · 2 files · checks passed</div></div><span class="meta">09:46</span></div>
        <div class="item seen"><span class="lead"><i data-i="check" class="sm muted"></i></span>
          <div class="grow"><div class="t ell">Gutter blame width</div><div class="meta ell" style="margin-top:2px">zed · answered · cargo test passed</div></div><span class="meta">09:31</span></div>
        <div class="item seen"><span class="lead"><i data-i="alert" class="sm" style="color:var(--fail)"></i></span>
          <div class="grow"><div class="t ell">VM snapshot restore</div><div class="meta ell" style="margin-top:2px">minivm · stopped: provider rate limit</div></div><span class="meta">09:15</span></div>
        <div class="item one" style="color:var(--accent)"><span class="lead"><i data-i="clock" class="sm"></i></span><span class="grow strong" style="font-size:14px">Earlier sessions</span><span class="meta">24</span><i data-i="chev-r" class="sm"></i></div>
      </div>
    </div>${START}`,

  // Working, on the streamed thread: the strip's run line is what happened in
  // order, so the failed type check shows where it was.
  working: `${BAR5("Qwen signatures", "pi · studio-mac")}
    <div class="scroll col g16" style="justify-content:flex-end;padding-bottom:12px">
      <div class="you">qwen3.8-flash on OpenCode returns empty thinking signatures and we reject the response. Accept empty signatures there, but keep the check strict for Anthropic.</div>
      <p class="say" style="margin-top:8px">I'll find where thinking signatures are checked.</p>
      <div class="calls">
        <div class="call two" style="--hue:var(--look)">
          <span class="ic"><i data-i="file"></i></span>
          <span class="what"><span class="ln"><b>Read</b> 5 files</span><span class="sub">openai-completions.ts, anthropic.ts, +3</span></span>
          <span class="tm">0:09</span><i data-i="chev-d"></i>
        </div>
        <div class="call" style="--hue:var(--look)">
          <span class="ic"><i data-i="search"></i></span>
          <span class="what"><span class="ln"><b>Searched</b> “signature” · 14 in 4 files</span></span>
          <span class="tm">0:02</span>
        </div>
      </div>
      <p class="say">The check in <code>readThinking</code> rejects an empty signature from every provider. I'll limit it to Anthropic and add a test.</p>
      <div class="calls">
        <div class="call" style="--hue:var(--change)">
          <span class="ic"><i data-i="pencil"></i></span>
          <span class="what"><span class="ln"><b>Edited</b> <code>openai-completions.ts</code></span></span>
          <span class="cmd add">+3</span><span class="cmd del">−1</span>
        </div>
        <div class="call" style="--hue:var(--change)">
          <span class="ic"><i data-i="plus"></i></span>
          <span class="what"><span class="ln"><b>Created</b> <code>qwen.test.ts</code></span></span>
          <span class="cmd add">+18</span>
        </div>
        <div class="call two err">
          <span class="ic"><i data-i="x"></i></span>
          <span class="what"><span class="ln"><b>Checked</b> <code>pnpm tsc --noEmit</code></span><span class="sub">Failed · qwen.test.ts:14 'model' is possibly undefined</span></span>
          <span class="tm">0:14</span><i data-i="chev-r"></i>
        </div>
        <div class="call" style="--hue:var(--change)">
          <span class="ic"><i data-i="pencil"></i></span>
          <span class="what"><span class="ln"><b>Edited</b> <code>qwen.test.ts</code></span></span>
          <span class="cmd add">+1</span><span class="cmd del">−1</span>
        </div>
      </div>
      <div class="open-call" style="--hue:var(--checking)">
        <div class="call">
          <span class="ic"><i data-i="term"></i></span>
          <span class="what"><span class="ln"><b>Checking</b> <code>pnpm test --filter @pi/ai</code></span></span>
          <span class="tm">0:22</span>
        </div>
<div class="tail"><span class="ok">✓</span> anthropic › rejects a missing signature
<span class="ok">✓</span> anthropic › keeps a valid signature
<span class="ok">✓</span> qwen › accepts an empty signature
<span class="now">▸ openai-completions › streams thinking</span><span class="caret" style="height:14px;vertical-align:-2px"></span></div>
        <div class="foot"><span class="bar-mini"><b style="width:78%;background:var(--checking)"></b></span><span class="meta">11 of 14</span></div>
      </div>
    </div>
    <div class="dock">
      <div class="strip">
        <span class="on-panel">${RUN(QWEN_NOW, { width: 92, total: 171, small: true, live: true })}</span>
        <span class="stage">Checking</span><span class="meta grow">2:51</span>
        <span class="open"><i data-i="chev-u"></i></span>
        <span class="btn sm quiet" style="color:var(--fail);padding:0 12px"><i data-i="stop" class="sm"></i>Stop</span>
      </div>${SLIM("Queue a follow-up…")}
    </div>`,

  // Done: the run as it happened, Pi's hand-off, what Pi thinks deserves a
  // look, and the report.
  done: `${BAR5("Qwen signatures", "pi · done in 4:31")}
    <div class="scroll col g24" style="padding-top:8px">
      <div class="you ell" style="max-width:86%">qwen3.8-flash on OpenCode returns empty thinking…</div>
      <div>
        ${RUN(QWEN, { width: 372, total: 271 })}
        <div class="rlkey">
          <span style="--hue:var(--look)"><i data-i="eye"></i>Looking <span class="tm">1:10</span></span>
          <span style="--hue:var(--change)"><i data-i="pencil"></i>Changing <span class="tm">1:35</span></span>
          <span style="--hue:var(--checking)"><i data-i="term"></i>Checking <span class="tm">1:46</span></span>
        </div>
      </div>
      <p class="headline">OpenCode models may now send an empty signature; Anthropic still must.</p>
      <div>
        <div class="label">Worth a look</div>
        <div class="card" style="margin-top:8px;padding:0 16px">
          <div class="look-row"><i data-i="flag"></i>
            <div class="grow"><div class="t">isAnthropic() <code>· line 242</code></div><div class="d">It trusts <code>model.provider</code>, so an Anthropic model behind a proxy would skip the check.</div></div>
            <i data-i="chev-r" class="go"></i></div>
          <div class="look-row"><i data-i="flag"></i>
            <div class="grow"><div class="t">DeepSeek isn't tested</div><div class="d">studio-mac has no DeepSeek key, so that path never ran.</div></div>
            <i data-i="chev-r" class="go"></i></div>
        </div>
      </div>
      <div class="card list" style="padding:0 16px">
        <div class="item one"><i data-i="file" class="sm muted"></i><span class="grow ell">openai-completions.ts</span>
          <span class="blocks"><span class="a"></span><span class="a"></span><span class="a"></span><span class="d"></span><span></span></span><span class="cmd add" style="width:24px;text-align:right">+3</span><span class="cmd del" style="width:20px;text-align:right">−1</span></div>
        <div class="item one"><i data-i="file" class="sm muted"></i><span class="grow ell">qwen.test.ts</span>
          <span class="blocks"><span class="a"></span><span class="a"></span><span class="a"></span><span class="a"></span><span class="a"></span></span><span class="cmd add" style="width:24px;text-align:right">+18</span><span style="width:20px"></span></div>
        <div class="item"><span class="mk pass" style="position:static;flex:none"><i data-i="check"></i></span>
          <div class="grow" style="min-width:0"><div class="cmd secondary ell">pnpm test --filter @pi/ai</div><div class="meta" style="margin-top:2px">tsc failed once, then passed</div></div>
          <span class="badge pass">Passed</span></div>
      </div>
    </div>
    <div class="dock g8">
      <span class="btn primary wide"><i data-i="diff" class="sm"></i>Review 2 changed files</span>${SLIM("Ask a follow-up…")}
    </div>`,
};

document.querySelectorAll("[data-v5]").forEach((el) => {
  el.insertAdjacentHTML("afterbegin", V5[el.dataset.v5] || "");
});
