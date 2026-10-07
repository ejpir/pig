// Screens shown in both themes: <main class="content" data-frag="working">.
// Load before next4.js, which draws the icons and the phone's chrome.
const COMPOSER = (placeholder) => `
      <div class="composer">
        <div class="draft ph">${placeholder}</div>
        <div class="tools"><span class="tap"><i data-i="clip"></i></span>
          <span class="control">Opus 5.5<i data-i="chev-d"></i></span><span class="control">High<i data-i="chev-d"></i></span>
          <span class="grow"></span><span class="send off"><i data-i="send"></i></span></div>
      </div>`;

const FRAG = {
  // Working: Pi has handed three searches to scouts running side by side;
  // the plan and the hand-off are still ahead.
  working: `
    <div class="bar">
      <span class="tap"><i data-i="back"></i></span>
      <div class="title"><b>Provider retries</b><span>pi · studio-mac</span></div>
      <span class="tap"><i data-i="info"></i></span>
      <span class="tap"><i data-i="dots"></i></span>
    </div>
    <div class="scroll col g24" style="padding-top:8px">
      <div class="you">Every provider retries differently. Use scouts to find how each one retries, then plan one shared helper.</div>
      <div class="col g24">
        <div class="st" style="--hue:var(--h-read)">
          <span class="stn"><i data-i="eye"></i></span><span class="to" style="--hue:var(--accent)"></span>
          <div class="body">
            <div class="row"><span class="strong grow">Understood</span><span class="meta">0:14</span></div>
            <div class="meta" style="margin-top:2px">Read 2 files · listed packages/ai/src/providers</div>
          </div>
        </div>
        <div class="st" style="--hue:var(--accent)">
          <span class="stn live"><i data-i="fork"></i></span><span class="to ahead"></span>
          <div class="body">
            <div class="row"><span class="strong grow">Handed off</span><span class="meta">1:02</span></div>
            <div class="meta" style="margin-top:2px">3 scouts in parallel · Haiku 4.5</div>
            <div class="card team" style="margin-top:12px">
              <div class="sub">
                <span class="tile scout"><i data-i="search"></i></span>
                <div class="grow">
                  <div class="who ell">Anthropic and Bedrock</div>
                  <div class="meta ell now"><i data-i="check" class="sm" style="color:var(--green);width:12px;height:12px;vertical-align:-1px"></i> 4 call sites, one backoff</div>
                </div>
                <div class="end"><span class="meta">0:48</span><span class="mini"><span class="r"></span><span class="r"></span><span class="h"></span></span></div>
                <i data-i="chev-r"></i>
              </div>
              <div class="sub">
                <span class="tile scout live"><i data-i="search"></i></span>
                <div class="grow">
                  <div class="who ell">OpenAI and OpenCode</div>
                  <div class="meta ell now">Searching “retryAfter”</div>
                </div>
                <div class="end"><span class="meta">1:02</span><span class="mini"><span class="r"></span><span class="r"></span><span></span></span></div>
                <i data-i="chev-r"></i>
              </div>
              <div class="sub">
                <span class="tile scout live"><i data-i="search"></i></span>
                <div class="grow">
                  <div class="who ell">Google and Mistral</div>
                  <div class="meta ell now">Reading google-gemini.ts</div>
                </div>
                <div class="end"><span class="meta">1:02</span><span class="mini"><span class="r"></span><span></span><span></span></span></div>
                <i data-i="chev-r"></i>
              </div>
            </div>
          </div>
        </div>
        <div class="st ahead">
          <span class="stn ahead"><i data-i="plan"></i></span><span class="to ahead"></span>
          <div class="body row g8"><span class="faint">Plan</span><span class="meta faint">planner, with what the scouts found</span></div>
        </div>
        <div class="st ahead">
          <span class="stn ahead"><i data-i="send"></i></span>
          <div class="body row g8"><span class="faint">Hand off</span><span class="meta faint">The plan</span></div>
        </div>
      </div>
    </div>
    <div class="dock">
      <div class="strip">
        <span class="dot ring" style="--hue:var(--accent)"></span>
        <span class="strong" style="font-size:14px">Waiting on 2 scouts</span><span class="meta grow">1:16</span>
        <span class="btn sm quiet" style="color:var(--coral);padding:0 12px"><i data-i="stop" class="sm"></i>Stop</span>
      </div>${COMPOSER("Queue a follow-up…")}
    </div>`,
};

document.querySelectorAll("[data-frag]").forEach((el) => {
  el.insertAdjacentHTML("afterbegin", FRAG[el.dataset.frag] || "");
});
