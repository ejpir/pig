// Home under the sheet, shared by the sheets here. Load before next4.js.
document.querySelectorAll("[data-under=home]").forEach((el) => {
  el.insertAdjacentHTML("afterbegin", `
    <div class="bar" style="padding-left:12px">
      <div class="row g12 grow">
        <span class="tile" style="--hue:var(--accent);width:40px;height:40px;border-radius:12px"><i data-i="pi"></i></span>
        <div class="grow"><div class="row g4"><b class="t3">studio-mac</b><i data-i="chev-d" class="sm muted"></i></div>
          <div class="row g8 meta"><span class="dot" style="--hue:var(--green);width:6px;height:6px"></span>Connected</div></div>
      </div>
      <span class="tap"><i data-i="search"></i></span><span class="tap"><i data-i="settings"></i></span>
    </div>
    <div class="scroll col" style="padding-top:12px">
      <div class="label">Working</div>
      <div class="item"><span class="lead"><span class="dot ring" style="--hue:var(--h-read)"></span></span>
        <div class="grow"><div class="row"><span class="t ell grow">Streaming retry</span><span class="meta">2:40</span></div>
        <div class="meta" style="margin-top:6px">pi · Changing · 2 files</div></div></div>
    </div>
    <div class="scrim"></div>`);
});
