// Stroke icons (24-unit grid) for <i data-i="name"></i>, stand-ins for
// assets/icons, plus the Android status bar, gesture handle and keyboard.
const P = {
  back: '<path d="M19 12H5M11 6l-6 6 6 6"/>',
  search: '<circle cx="11" cy="11" r="6.5"/><path d="m20 20-4.2-4.2"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  folder: '<path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
  file: '<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/>',
  "chev-r": '<path d="m9 6 6 6-6 6"/>',
  "chev-l": '<path d="m15 6-6 6 6 6"/>',
  "chev-d": '<path d="m6 9 6 6 6-6"/>',
  "chev-u": '<path d="m6 15 6-6 6 6"/>',
  check: '<path d="m5 12.5 4.5 4.5L19 7"/>',
  x: '<path d="M6 6l12 12M18 6 6 18"/>',
  pencil: '<path d="M4 20h4L19 9l-4-4L4 16z"/><path d="m13 7 4 4"/>',
  eye: '<path d="M2 12s3.5-7 10-7 10 7 10 7-3.5 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/>',
  stop: '<rect x="6.5" y="6.5" width="11" height="11" rx="2"/>',
  clock: '<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/>',
  restore: '<path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5"/>',
  alert: '<path d="M12 3 2 20h20z"/><path d="M12 10v4M12 17v.5"/>',
  clip: '<path d="m20 11-8.5 8.5a5 5 0 0 1-7-7L13 4a3.3 3.3 0 0 1 4.7 4.7L9.2 17.2a1.7 1.7 0 0 1-2.4-2.4L14.5 7"/>',
  slash: '<rect x="3" y="3" width="18" height="18" rx="4"/><path d="m14.5 7-5 10"/>',
  send: '<path d="M12 19V5M6 11l6-6 6 6"/>',
  spark: '<path d="M12 3v4M12 17v4M3 12h4M17 12h4M6 6l2.5 2.5M15.5 15.5 18 18M6 18l2.5-2.5M15.5 8.5 18 6"/>',
  dots: '<circle cx="12" cy="5" r="1.4"/><circle cx="12" cy="12" r="1.4"/><circle cx="12" cy="19" r="1.4"/>',
  settings: '<path d="M4 6h9M17 6h3M4 12h3M11 12h9M4 18h11M19 18h1"/><circle cx="15" cy="6" r="2"/><circle cx="9" cy="12" r="2"/><circle cx="17" cy="18" r="2"/>',
  info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v6M12 7.5v.5"/>',
  shield: '<path d="M12 3 5 6v5c0 4.5 3 8 7 10 4-2 7-5.5 7-10V6z"/><path d="m9 12 2 2 4-4"/>',
  computer: '<rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/>',
  server: '<rect x="4" y="4" width="16" height="7" rx="2"/><rect x="4" y="13" width="16" height="7" rx="2"/><path d="M8 7.5h.01M8 16.5h.01"/>',
  key: '<circle cx="8" cy="15" r="4"/><path d="m11 12 9-9M16 7l3 3"/>',
  copy: '<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"/>',
  diff: '<path d="M12 4v8M8 8h8M8 18h8"/>',
  hand: '<path d="M18 11V6a1.5 1.5 0 0 0-3 0v4M15 10V4.5a1.5 1.5 0 0 0-3 0V10M12 10V5.5a1.5 1.5 0 0 0-3 0V12M9 12V8.5a1.5 1.5 0 0 0-3 0V15a6 6 0 0 0 6 6h1a6 6 0 0 0 6-6v-4a1.5 1.5 0 0 0-3 0"/>',
  pi: '<path d="M5 8h14M9 8v10M15 8v7.5a2.5 2.5 0 0 0 2.5 2.5"/>',
  scan: '<path d="M4 8V6a2 2 0 0 1 2-2h2M16 4h2a2 2 0 0 1 2 2v2M20 16v2a2 2 0 0 1-2 2h-2M8 20H6a2 2 0 0 1-2-2v-2"/><path d="M8 12h8"/>',
  open: '<path d="M14 4h6v6M20 4l-9 9"/><path d="M18 14v4a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4"/>',
  refresh: '<path d="M20 12a8 8 0 1 1-2.3-5.7L20 8"/><path d="M20 3v5h-5"/>',
  code: '<path d="m8 7-5 5 5 5M16 7l5 5-5 5"/>',
  layers: '<path d="m12 3 9 5-9 5-9-5z"/><path d="m3 13 9 5 9-5"/>',
  wifi: '<path d="M2 9a15 15 0 0 1 20 0M5.5 12.5a10 10 0 0 1 13 0M9 16a5 5 0 0 1 6 0"/><circle cx="12" cy="19.5" r=".8"/>',
  signal: '<path d="M4 20v-3M9 20v-7M14 20V9M19 20V4"/>',
  battery: '<rect x="2" y="7" width="18" height="10" rx="2"/><path d="M22 11v2"/><rect x="4" y="9" width="11" height="6" rx="1" fill="currentColor" stroke="none"/>',
  term: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="m7 9 3 3-3 3M13 15h4"/>',
};

const svg = (name) =>
  `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">${P[name] || ""}</svg>`;

// Keyboard: <div class="kb" data-kb="deep,deepseek,deeper"></div>
document.querySelectorAll(".kb[data-kb]").forEach((kb) => {
  const row = (keys) => `<div class="keys">${keys.split("").map((k) => `<span class="key">${k}</span>`).join("")}</div>`;
  kb.innerHTML = `<div class="sugg">${kb.dataset.kb.split(",").map((w, n) => `<span${n === 1 ? ' style="font-weight:600"' : ""}>${w}</span>`).join("")}</div>
    ${row("qwertyuiop")}${row("asdfghjkl")}
    <div class="keys"><span class="key w"><i data-i="chev-u" class="sm"></i></span>${row("zxcvbnm").slice(18, -6)}<span class="key w"><i data-i="back" class="sm"></i></span></div>
    <div class="keys"><span class="key w">?123</span><span class="key">,</span><span class="key space"></span><span class="key">.</span><span class="key w act"><i data-i="chev-r" class="sm"></i></span></div>`;
});

document.querySelectorAll(".phone").forEach((phone) => {
  if (phone.dataset.chrome === "none") return;
  const status = document.createElement("div");
  status.className = "status";
  status.innerHTML = `<span>9:41</span><span class="sp"></span>${["signal", "wifi", "battery"].map((n) => `<i data-i="${n}"></i>`).join("")}`;
  phone.prepend(status);
  const gesture = document.createElement("div");
  gesture.className = "gesture";
  phone.append(gesture);
});

document.querySelectorAll("i[data-i]").forEach((el) => { el.innerHTML = svg(el.dataset.i); });
