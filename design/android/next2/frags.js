// Screens that appear under a sheet or in both themes: <main class="content" data-frag="home">.
// Load before next2.js, which draws the icons.
const DOCK = (placeholder, extra = "") => `
      <div class="composer">
        <div class="draft ph">${placeholder}</div>
        <div class="tools"><span class="tap"><i data-i="clip"></i></span>
          <span class="control">Opus 5.5<i data-i="chev-d"></i></span><span class="control">High<i data-i="chev-d"></i></span>
          <span class="grow"></span>${extra}<span class="send off"><i data-i="send"></i></span></div>
      </div>`;

const FRAG = {
  home: `
    <div class="bar" style="padding-left:16px">
      <div class="grow">
        <div class="row g8"><b class="title">studio-mac</b><i data-i="chev-d" class="sm muted"></i></div>
        <div class="row g8 small"><span style="width:6px;height:6px;border-radius:3px;background:var(--green)"></span>Connected</div>
      </div>
      <span class="tap"><i data-i="search"></i></span>
      <span class="tap"><i data-i="settings"></i></span>
    </div>
    <div class="scroll col" style="padding-top:12px">
      <div class="ask">
        <div class="row g8"><span class="strong grow ell">Qwen signatures</span><span class="small">2 min ago</span></div>
        <p class="pi q" style="margin-top:8px">Run the provider tests?</p>
        <div class="well cmd ell" style="margin-top:16px;padding:10px 12px;color:var(--secondary)">pnpm test --filter @pi/ai -- qwen</div>
        <div class="row" style="margin-top:16px"><span class="small grow">In pi</span><span class="btn sm primary">Answer</span></div>
      </div>

      <div class="col g24" style="margin-top:28px">
        <div>
          <div class="row"><span class="strong grow ell">Streaming retry</span><span class="small">2:40</span></div>
          <div class="run" style="margin-top:6px"><b class="read" style="width:31px"></b><b class="edit" style="width:166px"></b><span class="now" style="--hue:var(--edit)"></span></div>
          <div class="row small" style="margin-top:4px"><span class="grow">Changing 2 files</span><span>pi</span></div>
        </div>
        <div>
          <div class="row"><span class="strong grow ell">Gutter blame width</span><span class="small">0:52</span></div>
          <div class="run" style="margin-top:6px"><b class="read" style="width:15px"></b><b class="edit" style="width:31px"></b><b class="check" style="width:18px"></b><span class="now" style="--hue:var(--check)"></span></div>
          <div class="row small" style="margin-top:4px"><span class="grow">Verifying with cargo test</span><span>zed</span></div>
        </div>
      </div>

      <div class="rule" style="margin-top:24px"></div>
      <div class="rows">
        <div class="r64"><span class="glyph"><i data-i="check" class="sm" style="color:var(--green)"></i></span>
          <div class="grow"><div class="strong ell">Mistral thinking</div><div class="small ell" style="margin-top:2px">3 files changed, checks passed</div></div>
          <div class="small" style="text-align:right">14:02<br>pi</div></div>
        <div class="r64"><span class="glyph"><i data-i="check" class="sm" style="color:var(--green)"></i></span>
          <div class="grow"><div class="strong ell">Kimi K3 default</div><div class="small ell" style="margin-top:2px">Answered without changes</div></div>
          <div class="small" style="text-align:right">11:40<br>pi</div></div>
        <div class="r64"><span class="glyph"><i data-i="alert" class="sm" style="color:var(--coral)"></i></span>
          <div class="grow"><div class="strong ell">VM snapshot restore</div><div class="small ell" style="margin-top:2px">Stopped by a provider error</div></div>
          <div class="small" style="text-align:right">09:15<br>minivm</div></div>
        <div class="r56" style="color:var(--accent)"><span class="grow strong" style="font-size:14px">Earlier sessions</span><span class="small" style="color:inherit">24</span><i data-i="chev-r" class="sm"></i></div>
      </div>
    </div>
    <div class="dock">
      <div class="composer row" style="height:56px;padding:0 8px 0 20px;border-radius:28px">
        <span class="grow faint">What should we change?</span>
        <span class="chip" style="height:28px;margin-right:4px"><i data-i="folder"></i>pi</span>
        <span class="send"><i data-i="plus"></i></span>
      </div>
    </div>`,

  working: `
    <div class="bar">
      <span class="tap"><i data-i="back"></i></span>
      <div class="name"><b>Qwen signatures</b><span>pi on studio-mac</span></div>
      <span class="tap"><i data-i="info"></i></span>
      <span class="tap"><i data-i="dots"></i></span>
    </div>
    <div class="scroll col g32" style="padding-top:8px">
      <div class="you">qwen3.8-flash on OpenCode returns empty thinking signatures and we reject the response. Accept empty signatures there, but keep the check strict for Anthropic.</div>
      <div class="col g24">
        <div class="stage" style="--hue:var(--read)">
          <span class="stn"></span><span class="to"></span>
          <div>
            <div class="row"><span class="strong grow">Understood</span><span class="small">0:21</span></div>
            <div class="cmd muted" style="margin-top:4px">openai-completions.ts</div>
            <div class="cmd muted">“signature” in packages/ai</div>
          </div>
        </div>
        <div class="stage" style="--hue:var(--edit)">
          <span class="now"></span><span class="to ahead"></span>
          <div>
            <div class="row"><span class="strong grow">Changing</span><span class="add">+3</span><span class="del" style="width:28px;text-align:right">−1</span></div>
            <div class="cmd muted" style="margin-top:4px">openai-completions.ts</div>
            <div class="card code" style="margin-top:12px;padding:4px 0;border-radius:12px">
              <div class="l"><span class="n">211</span><span><span class="k">const</span> sig = block.signature;</span></div>
              <div class="l d"><span class="n">212</span><span><span class="k">if</span> (!sig) {</span></div>
              <div class="l a"><span class="n">212</span><span><span class="k">if</span> (!sig &amp;&amp; isAnthropic(model)) {</span></div>
            </div>
          </div>
        </div>
        <div class="stage ahead">
          <span class="stn ahead"></span><span class="to ahead" style="bottom:-20px"></span>
          <div class="what">Verify</div>
        </div>
        <div class="stage ahead">
          <span class="stn ahead"></span>
          <div class="what">Hand off</div>
        </div>
      </div>
    </div>
    <div class="dock">
      <div class="strip">
        <span class="strong">Working</span><span class="small grow">1:12</span>
        <span class="btn sm quiet" style="color:var(--coral);padding:0 12px"><i data-i="stop" class="sm"></i>Stop</span>
      </div>${DOCK("Queue a follow-up…")}
    </div>`,

  done: `
    <div class="bar">
      <span class="tap"><i data-i="back"></i></span>
      <div class="name"><b>Qwen signatures</b><span>Done in 4:31 on studio-mac</span></div>
      <span class="tap"><i data-i="info"></i></span>
      <span class="tap"><i data-i="dots"></i></span>
    </div>
    <div class="scroll col g24" style="padding-top:8px">
      <div class="you ell" style="max-width:86%">qwen3.8-flash on OpenCode returns empty thinking…</div>
      <div>
        <div class="run"><b class="read" style="width:54px"></b><b class="edit" style="width:159px"></b><b class="check" style="width:125px"></b><b class="hand" style="width:28px"></b></div>
        <div class="times"><span style="width:54px">0:40</span><span style="width:159px">1:58</span><span style="width:125px">1:32</span><span style="width:28px">0:21</span></div>
      </div>
      <div>
        <p class="pi q">OpenCode models may now send an empty signature; Anthropic still must.</p>
        <p class="prose" style="margin-top:12px">The check in <span class="cmd">readThinking</span> only rejects a missing signature for Anthropic models. A regression test covers both providers.</p>
      </div>
      <div class="rows" style="border-top:1px solid var(--line);border-bottom:1px solid var(--line)">
        <div class="r56"><span class="cmd grow ell">openai-completions.ts</span><span class="add">+3</span><span class="del" style="width:28px;text-align:right">−1</span></div>
        <div class="r56"><span class="cmd grow ell">qwen.test.ts</span><span class="add">+18</span><span style="width:28px"></span></div>
        <div class="r56"><span class="cmd grow ell secondary">pnpm test --filter @pi/ai</span><span class="strong" style="color:var(--green);font-size:14px">Passed</span></div>
      </div>
    </div>
    <div class="dock">
      <span class="btn primary wide">Review 2 changed files</span>${DOCK("Ask a follow-up…")}
    </div>`,
};

document.querySelectorAll("[data-frag]").forEach((el) => {
  el.insertAdjacentHTML("afterbegin", FRAG[el.dataset.frag] || "");
});
