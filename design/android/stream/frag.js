// Shared pieces for the streamed-calls mock. Load after ../../next4/frags.js
// (for COMPOSER) and before ../../next4/next4.js, which draws the icons.
//   <main class="content" data-stream="changing|verifying"> fills a thread
//   <div data-scrim></div> dims what is under a sheet
//   <div data-composer="placeholder"></div> is the composer

const BAR = (sub) => `
    <div class="bar">
      <span class="tap"><i data-i="back"></i></span>
      <div class="title"><b>Qwen signatures</b><span>${sub}</span></div>
      <span class="tap"><i data-i="info"></i></span>
      <span class="tap"><i data-i="dots"></i></span>
    </div>`;

// The run line folded into the dock: four segments, the stage's name, the time.
const DOCK = (segs, stage, hue, time) => `
    <div class="dock">
      <div class="strip">
        <span class="segs">${segs}</span>
        <span class="stage" style="color:var(${hue})">${stage}</span><span class="meta grow">${time}</span>
        <span class="open"><i data-i="chev-u"></i></span>
        <span class="btn sm quiet" style="color:var(--coral);padding:0 12px"><i data-i="stop" class="sm"></i>Stop</span>
      </div>${COMPOSER("Queue a follow-up…")}
    </div>`;

const READS = `
        <div class="call two" style="--hue:var(--h-read)">
          <span class="ic"><i data-i="file"></i></span>
          <span class="what"><span class="ln"><b>Read</b> 5 files</span><span class="sub">openai-completions.ts, anthropic.ts, +3</span></span>
          <span class="tm">0:09</span><i data-i="chev-d"></i>
        </div>
        <div class="call" style="--hue:var(--h-read)">
          <span class="ic"><i data-i="search"></i></span>
          <span class="what"><span class="ln"><b>Searched</b> “signature” · 14 in 4 files</span></span>
          <span class="tm">0:02</span>
        </div>`;

const STREAM = {
  // Pi is changing a file: what it read is folded, the edit is open and live.
  changing: `${BAR("pi · studio-mac")}
    <div class="scroll col g16" style="padding-top:8px">
      <div class="you">qwen3.8-flash on OpenCode returns empty thinking signatures and we reject the response. Accept empty signatures there, but keep the check strict for Anthropic.</div>
      <p class="say" style="margin-top:8px">I'll find where thinking signatures are checked.</p>
      <div class="calls">${READS}
      </div>
      <p class="say">The check in <code>readThinking</code> rejects an empty signature from every provider. I'll limit it to Anthropic.</p>
      <div class="open-call" style="--hue:var(--h-edit)">
        <div class="call">
          <span class="ic"><i data-i="pencil"></i></span>
          <span class="what"><span class="ln"><b>Editing</b> <code>openai-completions.ts</code></span></span>
          <span class="cmd add">+3</span><span class="cmd del">−1</span>
        </div>
        <div class="code" style="background:var(--canvas);border:1px solid var(--line);padding:4px 0">
          <div class="l"><span class="n">211</span><span><span class="k">const</span> sig = block.signature;</span></div>
          <div class="l d"><span class="n">212</span><span><span class="k">if</span> (!sig) {</span></div>
          <div class="l a"><span class="n">212</span><span><span class="k">if</span> (!sig &amp;&amp; isAnthropic(model)) {</span><span class="caret" style="height:15px;vertical-align:-2px"></span></div>
        </div>
      </div>
    </div>${DOCK('<span class="r"></span><span class="e live"></span><span></span><span></span>', "Changing", "--h-edit", "1:12")}`,

  // Pi is checking: the edits and a failed type check are folded, the test
  // run is open with its output streaming in. Scrolled to the newest call.
  verifying: `${BAR("pi · studio-mac")}
    <div class="scroll col g16" style="justify-content:flex-end;padding-bottom:12px">
      <div class="you">qwen3.8-flash on OpenCode returns empty thinking signatures and we reject the response. Accept empty signatures there, but keep the check strict for Anthropic.</div>
      <p class="say" style="margin-top:8px">I'll find where thinking signatures are checked.</p>
      <div class="calls">${READS}
      </div>
      <p class="say">The check in <code>readThinking</code> rejects an empty signature from every provider. I'll limit it to Anthropic and add a test.</p>
      <div class="calls">
        <div class="call" style="--hue:var(--h-edit)">
          <span class="ic"><i data-i="pencil"></i></span>
          <span class="what"><span class="ln"><b>Edited</b> <code>openai-completions.ts</code></span></span>
          <span class="cmd add">+3</span><span class="cmd del">−1</span>
        </div>
        <div class="call" style="--hue:var(--h-edit)">
          <span class="ic"><i data-i="plus"></i></span>
          <span class="what"><span class="ln"><b>Created</b> <code>qwen.test.ts</code></span></span>
          <span class="cmd add">+18</span>
        </div>
        <div class="call two err" style="--hue:var(--h-check)">
          <span class="ic"><i data-i="term"></i></span>
          <span class="what"><span class="ln"><b>Ran</b> <code>pnpm tsc --noEmit</code></span><span class="sub">Failed · qwen.test.ts:14 'model' is possibly undefined</span></span>
          <span class="tm">0:06</span><i data-i="chev-r"></i>
        </div>
        <div class="call" style="--hue:var(--h-edit)">
          <span class="ic"><i data-i="pencil"></i></span>
          <span class="what"><span class="ln"><b>Edited</b> <code>qwen.test.ts</code></span></span>
          <span class="cmd add">+1</span><span class="cmd del">−1</span>
        </div>
      </div>
      <div class="open-call" style="--hue:var(--h-check)">
        <div class="call">
          <span class="ic"><i data-i="term"></i></span>
          <span class="what"><span class="ln"><b>Running</b> <code>pnpm test --filter @pi/ai</code></span></span>
          <span class="tm">0:18</span>
        </div>
<div class="tail"><span class="ok">✓</span> anthropic › rejects a missing signature
<span class="ok">✓</span> anthropic › keeps a valid signature
<span class="ok">✓</span> qwen › accepts an empty signature
<span class="now">▸ openai-completions › streams thinking</span><span class="caret" style="height:14px;vertical-align:-2px"></span></div>
        <div class="foot"><span class="bar-mini"><b style="width:78%"></b></span><span class="meta">11 of 14</span></div>
      </div>
    </div>${DOCK('<span class="r"></span><span class="e"></span><span class="c live" style="box-shadow:0 0 0 2px color-mix(in srgb,var(--h-check) 25%,transparent)"></span><span></span>', "Verifying", "--h-check", "2:51")}`,
};

document.querySelectorAll("[data-stream]").forEach((el) => {
  el.insertAdjacentHTML("afterbegin", STREAM[el.dataset.stream] || "");
});
document.querySelectorAll("[data-composer]").forEach((el) => { el.outerHTML = COMPOSER(el.dataset.composer); });
document.querySelectorAll("[data-scrim]").forEach((el) => { el.className = "scrim"; });
