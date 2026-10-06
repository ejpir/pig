// Screens that appear under a sheet or in both themes: <main class="content" data-frag="home">.
// Load before next.js, which draws the icons.
const FRAG = {
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
    <div class="scroll col">
      <h1 class="headline" style="margin:16px 0 20px;font-size:26px;line-height:32px">1 needs you, 2 working</h1>

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
      <div class="list">
        <div class="item">
          <span class="lead"><span class="dot ring" style="--hue:var(--h-read)"></span></span>
          <div class="grow"><div class="t ell">Streaming retry</div>
            <div class="row g8" style="margin-top:6px"><span class="segs"><span class="r"></span><span class="e live"></span><span></span><span></span></span><span class="meta ell">Changing · 2 files</span></div></div>
          <span class="meta">2:40</span>
        </div>
        <div class="item">
          <span class="lead"><span class="dot ring" style="--hue:var(--h-read)"></span></span>
          <div class="grow"><div class="t ell">Gutter blame width</div>
            <div class="row g8" style="margin-top:6px"><span class="segs"><span class="r"></span><span class="e"></span><span class="c live" style="box-shadow:0 0 0 2px color-mix(in srgb,var(--h-check) 25%,transparent)"></span><span></span></span><span class="meta ell">zed · Verifying</span></div></div>
          <span class="meta">0:52</span>
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

  working: `
    <div class="bar">
      <span class="tap"><i data-i="back"></i></span>
      <div class="title"><b>Qwen signatures</b><span>pi · studio-mac</span></div>
      <span class="tap"><i data-i="info"></i></span>
      <span class="tap"><i data-i="dots"></i></span>
    </div>
    <div class="scroll col g24" style="padding-top:8px">
      <div class="you">
        <div class="meta" style="margin-bottom:4px">Turn 1 · 09:41</div>
        qwen3.8-flash on OpenCode returns empty thinking signatures and we reject the response. Accept empty signatures there, but keep the check strict for Anthropic.
      </div>
      <div class="col g24">
        <div class="stage">
          <span class="rail"></span>
          <span class="tile" style="--hue:var(--h-read)"><i data-i="eye"></i></span>
          <div class="body">
            <div class="row"><span class="strong grow">Understood</span><span class="meta">0:21</span></div>
            <div class="meta" style="margin-top:2px">Read 1 file · searched once</div>
            <div class="row g8 wrap" style="margin-top:12px">
              <span class="chip sm"><i data-i="file"></i>openai-completions.ts</span>
              <span class="chip sm"><i data-i="search"></i>“signature”</span>
            </div>
          </div>
        </div>
        <div class="stage">
          <span class="rail ahead"></span>
          <span class="tile live" style="--hue:var(--h-edit)"><i data-i="pencil"></i></span>
          <div class="body">
            <div class="row"><span class="strong grow">Changing</span><span class="cmd add">+3</span><span class="cmd del" style="margin-left:8px">−1</span></div>
            <div class="meta" style="margin-top:2px">openai-completions.ts</div>
            <div class="card code" style="margin-top:12px;padding:4px 0;border-radius:12px;overflow:hidden">
              <div class="l"><span class="n">211</span><span><span class="k">const</span> sig = block.signature;</span></div>
              <div class="l d"><span class="n">212</span><span><span class="k">if</span> (!sig) {</span></div>
              <div class="l a"><span class="n">212</span><span><span class="k">if</span> (!sig &amp;&amp; isAnthropic(model)) {</span></div>
            </div>
          </div>
        </div>
        <div class="stage todo">
          <span class="rail ahead" style="bottom:-24px"></span>
          <span class="tile todo"><i data-i="shield"></i></span>
          <div class="body row g8"><span class="t">Verify</span><span class="meta faint">Run the checks</span></div>
        </div>
        <div class="stage todo">
          <span class="tile todo"><i data-i="send"></i></span>
          <div class="body row g8"><span class="t">Hand off</span><span class="meta faint">Summary and changes</span></div>
        </div>
      </div>
    </div>
    <div class="dock">
      <div class="strip">
        <span class="dot ring" style="--hue:var(--h-read)"></span>
        <span class="grow ell" style="font-size:14px"><b style="font-weight:600">Working</b><span class="muted"> · editing a file · 1:12</span></span>
        <span class="btn sm quiet" style="color:var(--coral);padding:0 12px"><i data-i="stop" class="sm"></i>Stop</span>
      </div>
      <div class="composer">
        <div class="draft ph">Queue a follow-up…</div>
        <div class="tools"><span class="tap"><i data-i="clip"></i></span>
          <span class="control">Opus 5.5<i data-i="chev-d"></i></span><span class="control">High<i data-i="chev-d"></i></span>
          <span class="grow"></span><span class="send off"><i data-i="send"></i></span></div>
      </div>
    </div>`,

  done: `
    <div class="bar">
      <span class="tap"><i data-i="back"></i></span>
      <div class="title"><b>Qwen signatures</b><span>pi · done in 4:31</span></div>
      <span class="tap"><i data-i="info"></i></span>
      <span class="tap"><i data-i="dots"></i></span>
    </div>
    <div class="scroll col g24" style="padding-top:8px">
      <div class="you" style="max-width:86%">
        <div class="meta" style="margin-bottom:4px">Turn 1 · 09:41</div>
        <div class="ell">qwen3.8-flash on OpenCode returns empty thinking…</div>
      </div>
      <div class="row g8">
        <span class="tile" style="--hue:var(--h-read)"><i data-i="eye"></i></span>
        <span class="tile" style="--hue:var(--h-edit)"><i data-i="pencil"></i></span>
        <span class="tile" style="--hue:var(--h-check)"><i data-i="shield"></i></span>
        <span class="tile" style="--hue:var(--accent)"><i data-i="send"></i></span>
        <span class="meta grow" style="margin-left:4px">Read 2 · changed 2 · checked</span>
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
      <span class="btn primary wide"><i data-i="diff" class="sm"></i>Review 2 changed files</span>
      <div class="composer">
        <div class="draft ph">Ask a follow-up…</div>
        <div class="tools"><span class="tap"><i data-i="clip"></i></span>
          <span class="control">Opus 5.5<i data-i="chev-d"></i></span><span class="control">High<i data-i="chev-d"></i></span>
          <span class="grow"></span><span class="send off"><i data-i="send"></i></span></div>
      </div>
    </div>`,
};

document.querySelectorAll("[data-frag]").forEach((el) => {
  el.insertAdjacentHTML("afterbegin", FRAG[el.dataset.frag] || "");
});
