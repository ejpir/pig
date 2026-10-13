function refresh(s) {
    s.ring_label = "Fixed ring · " + fmt(s.ring);
    s.wheel_label = "Rolling circle · " + fmt(s.wheel);
    s.pen_label = "Pen distance · " + fmt(s.pen);
    s.speed_label = "Speed · " + fmt(s.speed) + "×";
    let a = Math.trunc(s.ring); let b = Math.trunc(s.wheel);
    while (b != 0) { let c = a % b; a = b; b = c; }
    s.turns = s.wheel / a;
    s.closure = "Closes after " + fmt(s.turns) + " turns · ratio " + fmt(s.ring) + ":" + fmt(s.wheel);
    s.status = "Paused · tap Play to explore";
    if (s.playing) { s.status = "Playing · drag the controls"; }
    return s;
}
function dispatch(s, action) {
    if (action == "play") { s.playing = !s.playing; }
    if (action == "restart" || action == "adjust") { s.angle = 0.0; s.progress = 0.0; }
    if (action == "reset") { s.ring = 96.0; s.wheel = 36.0; s.pen = 54.0; s.speed = 1.0; s.angle = 0.0; s.progress = 0.0; s.playing = false; }
    return refresh(s);
}
function tick(s, dt) {
    s.angle += dt * s.speed * 2.0;
    s.progress += dt * s.speed / 3.0;
    if (s.progress >= 1.0) { s.progress = 1.0; s.playing = false; }
    s.status = "Rolling · " + fmt(Math.floor((s.progress*100.0))) + "% traced";
    if (!s.playing) { s.status = "Complete · 100% traced"; }
    return s;
}
function pen_point(s, t, scale) {
    let k = s.ring - s.wheel;
    let q = k / s.wheel * t;
    return [(k*Math.cos(t) + s.pen*Math.cos(q))*scale, (k*Math.sin(t) - s.pen*Math.sin(q))*scale];
}
function scene(s) {
    let extent = s.ring - s.wheel + s.pen;
    if (s.ring > extent) { extent = s.ring; }
    let scale = 140.0 / (extent + 1.0);
    let k = s.ring - s.wheel;
    let center = [k*Math.cos(s.angle)*scale, k*Math.sin(s.angle)*scale];
    let pen = pen_point(s, s.angle, scale);
    let shapes = [{type:"circle",center:[0.0,0.0],radius:s.ring*scale,color:"#596574",width:1.0}];
    let ratio = k / s.wheel; let amplitude = s.pen; let interval = 6.28318530718 * s.turns / 128.0;
    let chunk = 0;
    while (chunk < 4) {
        let points = []; let i = 0;
        while (i < 33) {
            let t = (chunk*32+i) * interval; let q = ratio * t;
            points.push([(k*Math.cos(t)+amplitude*Math.cos(q))*scale, (k*Math.sin(t)-amplitude*Math.sin(q))*scale]); i += 1;
        }
        shapes.push({type:"polyline",points:points,color:"#77BFFF",width:1.5}); chunk += 1;
    }
    let target = Math.trunc((s.progress * 256.0));
    let start = 0;
    while (start < target) {
        let stop = start + 32;
        if (stop > target) { stop = target; }
        let points = []; let i = start;
        while (i <= stop) {
            let t = i / 256.0 * 6.28318530718 * s.turns;
            points.push(pen_point(s, t, scale)); i += 1;
        }
        shapes.push({type:"polyline",points:points,color:"#EAB879",width:2.0});
        start = stop;
    }
    shapes.push({type:"circle",center:center,radius:s.wheel*scale,color:"#EAB879",width:1.5});
    shapes.push({type:"line",from:center,to:pen,color:"#EAB879",width:2.0});
    shapes.push({type:"circle",center:pen,radius:3.5,color:"#EAB879",width:1.0,fill:"#EAB879"});
    return shapes;
}
function paint(g,shapes) {
 for(const p of shapes) {
 const o={color:p.color,width:p.width}; if(p.fill)o.fill=p.fill;
 if(p.type==='circle')g.circle(...p.center,p.radius,o);
 else if(p.type==='line')g.line(...p.from,...p.to,o);
 else if(p.type==='rect')g.rect(...p.origin,...p.size,o);
 else g.polyline(p.points,o);
 }
}

app({title:"Spirograph \u00b7 growing trace",state:{"ring": 96.0, "wheel": 36.0, "pen": 54.0, "speed": 1.0, "angle": 0.0, "turns": 3.0, "playing": false, "ring_label": "Fixed ring \u00b7 96", "wheel_label": "Rolling circle \u00b7 36", "pen_label": "Pen distance \u00b7 54", "speed_label": "Speed \u00b7 1\u00d7", "closure": "Closes after 3 turns \u00b7 ratio 96:36", "status": "Paused \u00b7 tap Play to explore", "progress": 0.0},view:ui.column([ui.text("SPIROGRAPH / MOTION LAB", {"style": "muted"}),
ui.text("A circle. An endless pattern.", {"style": "heading"}),
ui.text(s=>String(s["status"]), {"style": "muted"}),
ui.row([ui.button("Play / Pause",s=>{dispatch(s,"play");},{"tone": "accent", "id": "play"}),
ui.button("Restart",s=>{dispatch(s,"restart");},{"tone": "utility", "id": "restart"}),
ui.button("Reset",s=>{dispatch(s,"reset");},{"tone": "utility", "id": "reset"})], {"gap": 8}),
ui.canvas({"label": "Spirograph curve with moving rolling circle and pen", "height": 300, "viewBox": [-160, -160, 320, 320], "id": "scene",draw(g,s){paint(g,scene(s));}}),
ui.text(s=>String(s["closure"]), {"style": "history"}),
ui.text(s=>String(s["ring_label"]), {"style": "heading"}),
ui.slider("Fixed ring radius","ring",{"min": 60.0, "max": 140.0, "step": 1.0, "id": "adjust_ring",onChange(s){dispatch(s,"adjust");}}),
ui.text(s=>String(s["wheel_label"]), {"style": "heading"}),
ui.slider("Rolling circle radius","wheel",{"min": 8.0, "max": 58.0, "step": 1.0, "id": "adjust_wheel",onChange(s){dispatch(s,"adjust");}}),
ui.text(s=>String(s["pen_label"]), {"style": "heading"}),
ui.slider("Pen distance","pen",{"min": 0.0, "max": 120.0, "step": 1.0, "id": "adjust_pen",onChange(s){dispatch(s,"adjust");}}),
ui.text(s=>String(s["speed_label"]), {"style": "heading"}),
ui.slider("Animation speed","speed",{"min": 0.2, "max": 3.0, "step": 0.1, "id": "speed_speed",onChange(s){dispatch(s,"speed");}}),
ui.text("Blue: full pen path. Gold: rolling circle and pen. The wheel rolls inside the fixed ring; the pen follows the gold arm.", {"style": "muted"})], {"gap": 8, "width": null}),animation:{running:'playing',tick(s,dt){tick(s,dt);}}});
