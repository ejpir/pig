// Screens that appear under a sheet or in both themes: <main class="content" data-frag="home">.
// Load before next4.js, which draws the icons.
const COMPOSER = (placeholder, extra = "") => `
      <div class="composer">
        <div class="draft ph">${placeholder}</div>
        <div class="tools"><span class="tap"><i data-i="clip"></i></span>
          <span class="control">Opus 5.5<i data-i="chev-d"></i></span><span class="control">High<i data-i="chev-d"></i></span>
          <span class="grow"></span>${extra}<span class="send off"><i data-i="send"></i></span></div>
      </div>`;

const FRAG = {
  // Home: the question first, then working runs drawn to time, then today's results.
  home: `
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
    </div>
    <div class="scroll col" style="padding-top:12px">
      <div class="card" style="padding:16px;border-color:color-mix(in srgb,var(--h-wait) 45%,var(--line))">
        <div class="row g12">
          <span class="tile" style="--hue:var(--h-wait)"><i data-i="hand"></i></span>
          <div class="grow"><div class="strong ell">Qwen signatures</div><div class="meta">pi · asked 2 min ago</div></div>
        </div>
        <div style="margin-top:12px;line-height:22px">Run the provider tests?</div>
        <div class="row g8" style="margin-top:12px">
          <span class="well grow ell cmd" style="height:40px;line-height:40px;padding:0 12px;color:var(--secondary)">pnpm test --filter @pi/ai -- qwen</span>
          <span class="btn sm primary">Answer</span>
        </div>
      </div>

      <div class="label" style="margin:24px 0 0">Working</div>
      <div class="col">
        <div class="item">
          <span class="lead"><span class="dot ring" style="--hue:var(--h-read)"></span></span>
          <div class="grow"><div class="row"><span class="t ell grow">Streaming retry</span><span class="meta">2:40</span></div>
            <div class="track" style="margin-top:8px"><b class="r" style="width:8%"></b><b class="e live" style="width:45%;--hue:var(--h-edit)"></b></div>
            <div class="meta ell" style="margin-top:6px">pi · Changing · 2 files</div></div>
        </div>
        <div class="item">
          <span class="lead"><span class="dot ring" style="--hue:var(--h-read)"></span></span>
          <div class="grow"><div class="row"><span class="t ell grow">Gutter blame width</span><span class="meta">0:52</span></div>
            <div class="track" style="margin-top:8px"><b class="r" style="width:4%"></b><b class="e" style="width:8%"></b><b class="c live" style="width:5%;--hue:var(--h-check)"></b></div>
            <div class="meta ell" style="margin-top:6px">zed · Verifying · cargo test</div></div>
        </div>
      </div>

      <div class="label" style="margin:16px 0 0">Today</div>
      <div class="list">
        <div class="item"><span class="lead"><i data-i="check" class="sm" style="color:var(--green)"></i></span>
          <div class="grow"><div class="t ell">Mistral thinking</div><div class="meta ell" style="margin-top:2px">pi · 3 files · checks passed</div></div><span class="meta">14:02</span></div>
        <div class="item"><span class="lead"><i data-i="check" class="sm" style="color:var(--green)"></i></span>
          <div class="grow"><div class="t ell">Kimi K3 default</div><div class="meta ell" style="margin-top:2px">pi · answered, no changes</div></div><span class="meta">11:40</span></div>
        <div class="item"><span class="lead"><i data-i="alert" class="sm" style="color:var(--coral)"></i></span>
          <div class="grow"><div class="t ell">VM snapshot restore</div><div class="meta ell" style="margin-top:2px">minivm · stopped: provider error</div></div><span class="meta">09:15</span></div>
        <div class="item one" style="color:var(--accent)"><span class="lead"><i data-i="clock" class="sm"></i></span><span class="grow strong" style="font-size:14px">Earlier sessions</span><span class="meta">24</span><i data-i="chev-r" class="sm"></i></div>
      </div>
    </div>
    <div class="dock">
      <div class="composer row" style="height:56px;padding:0 8px 0 20px;border-radius:28px">
        <span class="grow faint">What should we change?</span>
        <span class="chip sm" style="margin-right:4px"><i data-i="folder"></i>pi</span>
        <span class="send"><i data-i="plus"></i></span>
      </div>
    </div>`,

  // Working: the run line down the gutter, a station per stage with its icon,
  // and what Pi read, searched and changed under each.
  working: `
    <div class="bar">
      <span class="tap"><i data-i="back"></i></span>
      <div class="title"><b>Qwen signatures</b><span>pi · studio-mac</span></div>
      <span class="tap"><i data-i="info"></i></span>
      <span class="tap"><i data-i="dots"></i></span>
    </div>
    <div class="scroll col g24" style="padding-top:8px">
      <div class="you">qwen3.8-flash on OpenCode returns empty thinking signatures and we reject the response. Accept empty signatures there, but keep the check strict for Anthropic.</div>
      <div class="col g24">
        <div class="st" style="--hue:var(--h-read)">
          <span class="stn"><i data-i="eye"></i></span><span class="to"></span>
          <div class="body">
            <div class="row"><span class="strong grow">Understood</span><span class="meta">0:21</span></div>
            <div class="meta" style="margin-top:2px">Read 5 files · searched twice</div>
            <div class="row g8 wrap" style="margin-top:12px">
              <span class="chip sm"><i data-i="file"></i>openai-completions.ts</span>
              <span class="chip sm"><i data-i="file"></i>anthropic.ts</span>
              <span class="chip sm"><i data-i="search"></i>“signature”</span>
              <span class="chip sm" style="color:var(--accent);font-weight:600">+4 more</span>
            </div>
          </div>
        </div>
        <div class="st" style="--hue:var(--h-edit)">
          <span class="stn live"><i data-i="pencil"></i></span><span class="to ahead"></span>
          <div class="body">
            <div class="row"><span class="strong grow">Changing</span><span class="cmd add">+3</span><span class="cmd del" style="margin-left:8px">−1</span></div>
            <div class="meta" style="margin-top:2px">Editing openai-completions.ts</div>
            <div class="card code" style="margin-top:12px;padding:4px 0;border-radius:12px;overflow:hidden">
              <div class="l"><span class="n">211</span><span><span class="k">const</span> sig = block.signature;</span></div>
              <div class="l d"><span class="n">212</span><span><span class="k">if</span> (!sig) {</span></div>
              <div class="l a"><span class="n">212</span><span><span class="k">if</span> (!sig &amp;&amp; isAnthropic(model)) {</span></div>
            </div>
          </div>
        </div>
        <div class="st ahead">
          <span class="stn ahead"><i data-i="shield"></i></span><span class="to ahead"></span>
          <div class="body row g8"><span class="faint">Verify</span><span class="meta faint">Run the checks</span></div>
        </div>
        <div class="st ahead">
          <span class="stn ahead"><i data-i="send"></i></span>
          <div class="body row g8"><span class="faint">Hand off</span><span class="meta faint">Summary and changes</span></div>
        </div>
      </div>
    </div>
    <div class="dock">
      <div class="strip">
        <span class="dot ring" style="--hue:var(--h-read)"></span>
        <span class="strong" style="font-size:14px">Working</span><span class="meta grow">1:12</span>
        <span class="btn sm quiet" style="color:var(--coral);padding:0 12px"><i data-i="stop" class="sm"></i>Stop</span>
      </div>${COMPOSER("Queue a follow-up…")}
    </div>`,

  // Done: the finished run line with each stretch's time, Pi's hand-off, and
  // the report card that says what changed and what was checked.
  done: `
    <div class="bar">
      <span class="tap"><i data-i="back"></i></span>
      <div class="title"><b>Qwen signatures</b><span>pi · done in 4:31</span></div>
      <span class="tap"><i data-i="info"></i></span>
      <span class="tap"><i data-i="dots"></i></span>
    </div>
    <div class="scroll col g24" style="padding-top:8px">
      <div class="you ell" style="max-width:86%">qwen3.8-flash on OpenCode returns empty thinking…</div>
      <div>
        <div class="run"><b class="r" style="width:54px"></b><b class="e" style="width:159px"></b><b class="c" style="width:125px"></b><b class="h" style="width:28px"></b></div>
        <div class="times"><span style="width:54px">0:40</span><span style="width:159px">1:58</span><span style="width:125px">1:32</span><span style="width:28px">0:21</span></div>
      </div>
      <div>
        <p class="headline">OpenCode models may now send an empty signature; Anthropic still must.</p>
        <p class="prose" style="margin-top:12px">The check in <code>readThinking</code> only rejects a missing signature for Anthropic models. A regression test covers both providers.</p>
      </div>
      <div class="card list" style="padding:0 16px">
        <div class="item one"><i data-i="file" class="sm muted"></i><span class="grow ell">openai-completions.ts</span>
          <span class="blocks"><span class="a"></span><span class="a"></span><span class="a"></span><span class="d"></span><span></span></span><span class="cmd add" style="width:24px;text-align:right">+3</span><span class="cmd del" style="width:20px;text-align:right">−1</span></div>
        <div class="item one"><i data-i="file" class="sm muted"></i><span class="grow ell">qwen.test.ts</span>
          <span class="blocks"><span class="a"></span><span class="a"></span><span class="a"></span><span class="a"></span><span class="a"></span></span><span class="cmd add" style="width:24px;text-align:right">+18</span><span style="width:20px"></span></div>
        <div class="item one"><i data-i="shield" class="sm" style="color:var(--green)"></i><span class="grow ell cmd secondary">pnpm test --filter @pi/ai</span><span class="badge" style="--hue:var(--green)">Passed</span></div>
      </div>
    </div>
    <div class="dock g8">
      <span class="btn primary wide"><i data-i="diff" class="sm"></i>Review 2 changed files</span>${COMPOSER("Ask a follow-up…")}
    </div>`,
};

document.querySelectorAll("[data-frag]").forEach((el) => {
  el.insertAdjacentHTML("afterbegin", FRAG[el.dataset.frag] || "");
});
