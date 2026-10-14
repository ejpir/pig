// Inline vector data only: no downloaded assets, images, scripts, or CSS.
const postcard = ui.svg(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 160 120">
  <title>Native SVG mountain postcard</title>
  <defs>
    <linearGradient id="sky" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#203A69"/><stop offset="1" stop-color="#79C8D0"/>
    </linearGradient>
    <linearGradient id="ridge" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="#EFFCF1"/><stop offset="1" stop-color="#91CFBF"/>
    </linearGradient>
  </defs>
  <rect x="4" y="4" width="152" height="112" rx="9" fill="url(#sky)" stroke="#C2E8DE" stroke-width="2"/>
  <circle cx="119" cy="32" r="13" fill="#FFE1A0"/>
  <g transform="translate(12 35)" fill="url(#ridge)">
    <path d="M0 58 L38 3 66 39 82 21 136 58 Z"/>
    <path d="M28 18 L38 3 51 20 39 16 Z" fill="#FFFFFF"/>
  </g>
  <path d="M15 99 Q40 91 65 99 T115 99 Q133 94 145 99" fill="none" stroke="#C1F6EF" stroke-width="2"/>
  <path d="M19 107 Q44 99 69 107 T119 107" fill="none" stroke="#80D7D7" stroke-width="1.5"/>
</svg>`);
const flourish = ui.path('M12 112 C20 105 28 119 36 112 S52 105 60 112');
app({
  title: 'SVG postcard',
  state: {angle: -8, zoom: 1, playing: false},
  view: ui.column([
    ui.text('Native vectors · rotate and resize'),
    ui.canvas({id:'postcard',label:'SVG mountain postcard',height:280,viewBox:[0,0,160,120],
      draw(g,s) {
        g.group({translate:[80,55],rotate:s.angle},g=>{
          g.svg(postcard,{x:-64*s.zoom,y:-48*s.zoom,width:128*s.zoom,height:96*s.zoom});
        });
        g.path(flourish,{color:'#5DAFA1',width:1.5});
        g.text('SVG / native',76,108,{size:12,color:'#4B766E',font:'mono'});
      }
    }),
    ui.slider('Rotate','angle',{min:-30,max:30,step:1}),
    ui.slider('Zoom','zoom',{min:0.65,max:1.15,step:0.05}),
    ui.row([
      ui.button('Play / Pause',s=>{s.playing=!s.playing;},{tone:'accent'}),
      ui.button('Reset',s=>{s.angle=-8;s.zoom=1;s.playing=false;})
    ])
  ]),
  animation:{running:'playing',tick(s,dt){s.angle=s.angle+dt*8;if(s.angle>30)s.angle=-30;}}
});
