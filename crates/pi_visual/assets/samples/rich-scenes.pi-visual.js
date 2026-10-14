// No assets or networking: reusable silhouette, cached scenery, gradient and HUD.
const ship = ui.path([
  ['M', 0, -12], ['C', 5, -7, 5, 0, 10, 8],
  ['Q', 0, 4, -10, 8], ['C', -5, 0, -5, -7, 0, -12], ['Z']
]);
app({
  title: 'Native flight',
  state: {x: 50, angle: 0, contacts: 0, playing: false},
  view: ui.column([
    ui.text('Hold Left / Right or focus the scene and use arrow keys.'),
    ui.canvas({
      id: 'flight', label: 'Flight scene; touch with multiple fingers', height: 280,
      viewBox: [0, 0, 100, 100],
      layers: [
        {deps: [], draw(g) {
          g.rect(0, 0, 100, 100, {color: '#15273F', fill: ui.linearGradient(180, '#15273F', '#325A71')});
          for (let i = 0; i < 12; i++) g.circle((i * 37 + 11) % 100, (i * 23 + 7) % 100, 0.6, {fill: '#FFFFFF'});
        }},
        {deps: ['x', 'angle'], draw(g, s) {
          g.group({translate: [s.x, 55], rotate: s.angle}, g => {
            g.path(ship, {fill: ui.linearGradient(90, '#70B8FF', '#B0EFFF'), color: '#E8F5FF'});
          });
        }},
        {deps: ['contacts'], draw(g, s) {
          g.text('Contacts: ' + String(s.contacts), 5, 5, {size: 16, color: '#FFFFFF', font: 'mono'});
        }}
      ],
      onInput(s, e, input) {
        s.contacts = input.pointers.filter(p => p.target === 'flight-input').length;
        if (e.kind === 'pointer' && (e.phase === 'down' || e.phase === 'move')) s.x = Math.max(12, Math.min(88, e.x));
      },
      inputId: 'flight-input'
    }),
    ui.row([ui.holdButton('Left', 'left'), ui.holdButton('Right', 'right')]),
    ui.row([
      ui.button('Play / Pause', s => {s.playing = !s.playing;}, {tone: 'accent'}),
      ui.button('Restart', s => {s.x = 50; s.angle = 0; s.playing = false;})
    ])
  ]),
  animation: {running: 'playing', tick(s, dt, input) {
    const direction = (input.buttons.includes('right') || input.keys.includes('right') ? 1 : 0)
      - (input.buttons.includes('left') || input.keys.includes('left') ? 1 : 0);
    s.x = Math.max(12, Math.min(88, s.x + direction * dt * 35));
    s.angle = direction * 15;
  }}
});
