// Icons next4.js doesn't have, drawn the same way. Load after next4.js.
const MORE = {
  // Work handed off: one line splitting into three.
  fork: '<path d="M3 12h6M9 12c3.5 0 4-6.5 8-6.5h4M9 12h12M9 12c3.5 0 4 6.5 8 6.5h4"/>',
  // A plan: a list with ticks.
  plan: '<path d="M10 6h10M10 12h10M10 18h10"/><path d="m3.5 6 1.5 1.5L7.5 5M3.5 12l1.5 1.5L7.5 11"/><circle cx="5.5" cy="18" r="1.2"/>',
  coin: '<circle cx="12" cy="12" r="8.5"/><path d="M14.5 9.5c-.5-1-1.4-1.5-2.5-1.5-1.5 0-2.5.8-2.5 2s1 1.6 2.5 2 2.5.9 2.5 2-1 2-2.5 2c-1.2 0-2.1-.5-2.6-1.5M12 6.5V8M12 16v1.5"/>',
  person: '<circle cx="12" cy="8" r="3.5"/><path d="M5 20a7 7 0 0 1 14 0"/>',
};
document.querySelectorAll("i[data-i]").forEach((el) => {
  const path = MORE[el.dataset.i];
  if (path) el.innerHTML = `<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">${path}</svg>`;
});
