// Icons next4.js doesn't have, drawn the same way, and the status bar's Live
// Update chip (<div class="phone" data-live="3:12">). Load after next4.js.
const MORE = {
  // Pi's pointer to what deserves a look.
  flag: '<path d="M5 21V4M5 4h11.5l-2.5 4 2.5 4H5"/>',
  phone: '<rect x="6.5" y="2.5" width="11" height="19" rx="2.5"/><path d="M10.5 18.5h3"/>',
  share: '<circle cx="18" cy="5.5" r="2.5"/><circle cx="6" cy="12" r="2.5"/><circle cx="18" cy="18.5" r="2.5"/><path d="m8.3 10.8 7.4-4.1M8.3 13.2l7.4 4.1"/>',
  inbox: '<path d="M3 13h5l1.5 3h5l1.5-3h5"/><path d="M5.5 5h13l2.5 8v5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-5z"/>',
  undo: '<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11"/>',
  image: '<rect x="3" y="4" width="18" height="16" rx="2"/><circle cx="9" cy="10" r="1.8"/><path d="m21 16-5-5-9 9"/>',
  message: '<path d="M4 5h16v11H9.5L5 20v-4H4z"/>',
  tabs: '<rect x="4" y="4" width="16" height="16" rx="3"/>',
};
Object.assign(P, MORE);

document.querySelectorAll(".phone[data-live]").forEach((phone) => {
  phone.querySelector(".status span").insertAdjacentHTML("afterend",
    `<span class="chip-live"><i data-i="pi"></i>${phone.dataset.live}</span>`);
});
document.querySelectorAll("i[data-i]").forEach((el) => {
  if (MORE[el.dataset.i] || !el.innerHTML) el.innerHTML = svg(el.dataset.i);
});
