// Thread content shared by several screens: <main class="content" data-frag="working">.
// phone.js inserts these before drawing icons.
window.FRAG = {
  working: `
    <div class="appbar">
      <span class="tap"><i data-i="back"></i></span>
      <div class="title"><b>Qwen signatures</b><span>pi · studio-mac</span></div>
      <span class="tap"><i data-i="info"></i></span>
      <span class="tap"><i data-i="dots"></i></span>
    </div>
    <div class="scroll col gap16" style="padding-top:6px">
      <div class="user-turn">
        <div class="label">You · Turn 1 · 09:41</div>
        <p style="margin-top:4px">qwen3.8-flash on OpenCode returns empty thinking signatures and we reject the response. Accept empty signatures there, but keep the check strict for Anthropic.</p>
      </div>
      <div class="col gap16" style="margin-top:4px">
        <div class="stage">
          <span class="rail-line"></span>
          <span class="tile" style="--hue:var(--h-read)"><i data-i="eye"></i></span>
          <div class="body"><div class="name">Understood</div><div class="what">Read 1 file · 1 search</div>
            <div class="row gap8" style="margin-top:8px;flex-wrap:wrap"><span class="chip"><i data-i="file" class="xs"></i>openai-completions.ts</span><span class="chip"><i data-i="search" class="xs"></i>“signature” in packages/ai</span></div>
          </div>
        </div>
        <div class="stage">
          <span class="rail-line" style="border-left:1px dashed var(--line-strong);background:none"></span>
          <span class="tile live" style="--hue:var(--h-edit)"><i data-i="pencil"></i></span>
          <div class="body">
            <div class="row"><span class="name grow">Changing</span><span class="mono add" style="font-size:12px">+3</span><span class="mono del" style="font-size:12px;margin-left:6px">−1</span></div>
            <div class="what">Editing openai-completions.ts</div>
            <div class="card code" style="margin-top:10px;padding:6px 0;border-radius:12px">
              <div class="l"><span class="ln">211</span><span><span class="k">const</span> signature = block.signature;</span></div>
              <div class="l del"><span class="ln">212</span><span><span class="k">if</span> (!signature) {</span></div>
              <div class="l add"><span class="ln">212</span><span><span class="k">if</span> (!signature &amp;&amp; isAnthropic(model)) {</span></div>
            </div>
          </div>
        </div>
        <div class="stage planned">
          <span class="rail-line" style="border-left:1px dashed var(--line-strong);background:none"></span>
          <span class="tile planned"><i data-i="shield"></i></span>
          <div class="body"><div class="name">Verify</div><div class="what faint">Run the checks</div></div>
        </div>
        <div class="stage planned">
          <span class="tile planned"><i data-i="send"></i></span>
          <div class="body"><div class="name">Hand off</div><div class="what faint">Summary and changes</div></div>
        </div>
      </div>
    </div>
    <div style="padding:10px 16px 8px" class="row gap12">
      <span class="dot live"></span>
      <div class="grow"><div style="font-size:14px;font-weight:600">Working</div><div class="hint" style="margin-top:-2px">Editing a file · 1:12</div></div>
    </div>
    <div class="composer">
      <div class="draft ph">Queue a follow-up…</div>
      <div class="tools"><span class="tap"><i data-i="clip"></i></span>
        <span class="composer-control"><span class="ell">Opus 5.5</span><i data-i="chev-d" class="xs"></i></span>
        <span class="composer-control">High<i data-i="chev-d" class="xs"></i></span><span class="grow"></span>
        <span class="tap" style="color:var(--coral)"><i data-i="stop"></i></span><span class="send off"><i data-i="send"></i></span>
      </div>
    </div>`,

  done: `
    <div class="appbar">
      <span class="tap"><i data-i="back"></i></span>
      <div class="title"><b>Qwen signatures</b><span>pi · done in 4:31</span></div>
      <span class="tap"><i data-i="info"></i></span>
      <span class="tap"><i data-i="dots"></i></span>
    </div>
    <div class="scroll col gap16" style="padding-top:6px">
      <div class="row gap8">
        <span class="tile" style="--hue:var(--h-read)"><i data-i="eye"></i></span>
        <span class="tile" style="--hue:var(--h-edit)"><i data-i="pencil"></i></span>
        <span class="tile" style="--hue:var(--h-check)"><i data-i="shield"></i></span>
        <span class="tile" style="--hue:var(--accent)"><i data-i="send"></i></span>
        <span class="hint" style="margin-left:4px">Read 2 · changed 2 · checked</span>
      </div>
      <div>
        <p class="serif" style="font-size:21px;line-height:1.3;font-style:italic">OpenCode models may now send an empty signature; Anthropic still must.</p>
        <p style="margin-top:10px;color:var(--secondary)">The check in <span class="mono" style="font-size:13px">readThinking</span> only rejects a missing signature for Anthropic models. A regression test covers both providers, and the provider tests pass.</p>
      </div>
      <div class="card">
        <div class="listrow"><i data-i="file" class="sm muted"></i><span class="grow ell">openai-completions.ts</span><span class="mono add" style="font-size:12px">+3</span><span class="mono del" style="font-size:12px">−1</span></div>
        <div class="listrow"><i data-i="file" class="sm muted"></i><span class="grow ell">qwen.test.ts</span><span class="mono add" style="font-size:12px">+18</span></div>
        <div class="listrow"><i data-i="shield" class="sm" style="color:var(--green)"></i><span class="grow">pnpm test --filter @pi/ai</span><span class="hint">passed</span></div>
      </div>
      <span class="btn primary wide"><i data-i="diff" class="sm"></i>Review 2 changed files</span>
    </div>
    <div class="composer">
      <div class="draft ph">Ask a follow-up…</div>
      <div class="tools"><span class="tap"><i data-i="clip"></i></span>
        <span class="composer-control"><span class="ell">Opus 5.5</span><i data-i="chev-d" class="xs"></i></span>
        <span class="composer-control">High<i data-i="chev-d" class="xs"></i></span><span class="grow"></span>
        <span class="send off"><i data-i="send"></i></span>
      </div>
    </div>`,
};
