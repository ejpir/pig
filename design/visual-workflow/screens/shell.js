// Wraps <main class="main"> in the shared title bar, sidebar and footer.
(() => {
  const main = document.querySelector("main.main");
  const status = document.body.dataset.status || "working"; // glyph for the active session
  const footer = document.body.dataset.footer || "Working · 2 observed edits · $0.41 · 31% context";
  const glyph = {
    working: '<span class="sb-live"></span>',
    waiting: '<span class="sb-wait"></span>',
    done: '<i data-i="check" style="color:var(--h-ok)"></i>',
  }[status];
  const win = document.createElement("div");
  win.className = "win";
  win.innerHTML = `
    <div class="titlebar"><i data-i="panel-left"></i><i data-i="search"></i><span class="sp"></span><i data-i="plus"></i><i data-i="panel-right"></i></div>
    <div class="body">
      <aside class="sidebar">
        <div class="sb-h"><span>Projects</span><i data-i="plus"></i></div>
        <div class="sb-row"><i data-i="chev-d"></i><i data-i="folder"></i><b>pi</b></div>
        <div class="sb-row indent sel"><i data-i="chat"></i>Qwen signatures<span class="tag">${glyph}</span></div>
        <div class="sb-row indent"><i data-i="chat"></i>Streaming retry<span class="tag"><span class="sb-wait"></span></span></div>
        <div class="sb-row indent"><i data-i="chat"></i>LSP shutdown<span class="tag">2h</span></div>
        <div class="sb-more">12 saved sessions…</div>
        <div class="sb-row"><i data-i="chev-r"></i><i data-i="folder"></i>zed</div>
        <div class="sb-row"><i data-i="chev-r"></i><i data-i="folder"></i>minivm</div>
        <div class="sb-row muted"><i data-i="plus"></i>Open folder…</div>
        <div class="sb-sp"></div>
        <div class="sb-row"><i data-i="gear"></i>Settings &amp; tools</div>
      </aside>
    </div>
    <div class="footer"><span class="dot"></span>Local process<span class="sp"></span><span class="mono">${footer}</span></div>`;
  main.replaceWith(win);
  win.querySelector(".body").appendChild(main);
  const s = document.createElement("style");
  s.textContent = `.sb-live{display:inline-block;width:8px;height:8px;border-radius:4px;background:var(--h-read);box-shadow:0 0 0 3px color-mix(in srgb,var(--h-read) 22%,transparent)}
  .sb-wait{display:inline-block;width:8px;height:8px;border-radius:4px;background:var(--h-wait)}`;
  document.head.appendChild(s);
})();
