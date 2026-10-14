const cube = ui.box(1.4, 1.4, 1.4);
const ball = ui.sphere(0.9, {segments: 32});
const ring = ui.torus(1.05, 0.28, {segments: 32});
const cone = ui.cone(0.8, 1.6, {segments: 24});
function circle(radius, y) {
  const points = [];
  for (let i = 0; i <= 64; i++) {
    const a = i / 64 * Math.PI * 2;
    points.push([Math.cos(a) * radius, y, Math.sin(a) * radius]);
  }
  return points;
}
app({title: "Solids · 3D", state: {angle: 0, yaw: 35, pitch: 22, speed: 1, playing: false, lastX: 0, lastY: 0},
  view: ui.column([
    ui.text("SOLIDS / NATIVE 3D", {style: "muted"}),
    ui.text(s => "Yaw " + s.yaw.toFixed(0) + "° · pitch " + s.pitch.toFixed(0) + "°", {style: "heading"}),
    ui.canvas({label: "Spinning solids; drag to orbit", height: 320, viewBox: [0, 0, 160, 100],
      draw(g, s) {
        g.scene3d({camera: {fov: 42, distance: 9, yaw: s.yaw, pitch: s.pitch}}, d => {
          d.line(circle(3.6, -1.6), {color: "#5A6B7C", width: 1.5});
          d.mesh(cube, {position: [-2.7, 0, 0], rotate: [s.angle, s.angle * 1.4, 0], color: "#4D9ABF"});
          d.mesh(ball, {position: [0, 0, 0], color: "#F09082"});
          d.mesh(ring, {position: [2.7, 0, 0], rotate: [70, s.angle * 2, 0], color: "#F1BE58"});
          d.mesh(cone, {position: [0, 0, -2.7], rotate: [0, s.angle, 0], color: "#7FC08A"});
          d.line([[0, -1.6, 0], [0, 2.2, 0]], {color: "#9AA8B6", width: 1});
        });
      },
      onInput(s, event) {
        if (event.kind !== "pointer") return;
        if (event.phase === "move") {
          s.yaw = (s.yaw - (event.x - s.lastX) * 2 + 360) % 360;
          s.pitch = Math.max(-80, Math.min(80, s.pitch + (event.y - s.lastY) * 2));
        }
        s.lastX = event.x;
        s.lastY = event.y;
      }}),
    ui.text("Drag to orbit. Triangles are shaded and depth-sorted natively.", {style: "muted"}),
    ui.slider("Speed", "speed", {min: 0, max: 3, step: 0.1}),
    ui.row([
      ui.button("Play / Pause", s => { s.playing = !s.playing; }, {tone: "accent"}),
      ui.button("Reset view", s => { s.yaw = 35; s.pitch = 22; s.angle = 0; })
    ])
  ]),
  animation: {running: "playing", tick(s, dt) { s.angle = (s.angle + dt * 60 * s.speed) % 360; }}});
