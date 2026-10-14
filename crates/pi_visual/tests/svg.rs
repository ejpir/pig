use pi_visual::{
    Runtime, Shape,
    graphics::{Paint, PathCommand as P},
};
use serde_json::json;
fn drawing(code: &str) -> String {
    format!("app({{title:'SVG',state:{{}},view:ui.canvas({{id:'s',draw(g){{{code}}}}})}});")
}
fn svg_scene(xml: &str) -> anyhow::Result<std::sync::Arc<pi_visual::Scene>> {
    Runtime::from_bundle(&drawing(&format!("g.svg(ui.svg({}));", json!(xml))))?.scene("s")
}
fn path(data: &str) -> Vec<P> {
    let scene = Runtime::from_bundle(&drawing(&format!(
        "g.path(ui.path({}),{{fill:'#FFFFFF'}})",
        json!(data)
    )))
    .unwrap()
    .scene("s")
    .unwrap();
    let Shape::Path { commands, .. } = &scene[0] else {
        panic!()
    };
    commands.clone()
}
#[test]
fn svg_path_syntax_relative_repetition_reflection_and_close() {
    let p = path("m1 2 3 4 h5 v6 q2 3 4 5 t6 7 c1 2 3 4 5 6 s7 8 9 10 z h2");
    assert_eq!(p[0], P::Move { to: [1., 2.] });
    assert_eq!(p[1], P::Line { to: [4., 6.] });
    assert_eq!(p[2], P::Line { to: [9., 6.] });
    assert_eq!(p[3], P::Line { to: [9., 12.] });
    assert_eq!(
        p[5],
        P::Quad {
            control: [15., 19.],
            to: [19., 24.]
        }
    );
    assert_eq!(
        p[7],
        P::Cubic {
            control1: [26., 32.],
            control2: [31., 38.],
            to: [33., 40.]
        }
    );
    assert_eq!(
        &p[8..],
        &[P::Close, P::Move { to: [1., 2.] }, P::Line { to: [3., 2.] }]
    );
    assert_eq!(
        path("M.5-.5L1e1 2E1ZZ"),
        vec![
            P::Move { to: [0.5, -0.5] },
            P::Line { to: [10., 20.] },
            P::Close
        ]
    );
}
#[test]
fn arc_flags_rotation_radii_correction_degeneracy_and_smooth_reset() {
    for flag in ["0 0", "0 1", "1 0", "1 1"] {
        let p = path(&format!("M10 10 A3 4 30 {flag} 40 30 s2 3 4 5"));
        assert!((3..=10).contains(&p.len()));
        let P::Cubic { control1, .. } = p.last().unwrap() else {
            panic!()
        };
        assert_eq!(*control1, [40., 30.]);
        let P::Cubic { to, .. } = p[p.len() - 2] else {
            panic!()
        };
        assert_eq!(to, [40., 30.]);
    }
    assert_eq!(
        path("M0 0 A0 2 0 0 0 5 6 A3 3 0 1 1 5 6"),
        vec![P::Move { to: [0., 0.] }, P::Line { to: [5., 6.] }]
    );
    assert_eq!(
        path("M0 0 A5 5 0 0 1 10 0 T20 0").last(),
        Some(&P::Quad {
            control: [10., 0.],
            to: [20., 0.]
        })
    );
}
#[test]
fn svg_defaults_inheritance_shapes_transform_and_source_unit_strokes() {
    let xml = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><title>Vector &amp; art</title><g fill="none" stroke="#abc" stroke-width="2" transform="translate(10 20) rotate(90)"><path d="M0 0H10V10Z"/><line x2="8" y2="9"/></g><rect x="1" y="2" width="3" height="4"/><polyline points="1,2 3,4"/><polygon points="2,3 4,5 6,2"/></svg>"##;
    let scene = svg_scene(xml).unwrap();
    let Shape::Group { children, .. } = &scene[0] else {
        panic!()
    };
    let Shape::Group {
        children: g,
        transform,
    } = &children[0]
    else {
        panic!()
    };
    assert_eq!(transform[4..], [10., 20.]);
    assert!(transform[0].abs() < 1e-6 && transform[1] == 1.);
    let Shape::Path {
        color,
        width,
        fill,
        scale_stroke,
        ..
    } = &g[0]
    else {
        panic!()
    };
    assert_eq!(*width, 2.);
    assert!(*scale_stroke && fill.is_none());
    assert_eq!(color, &Some(Paint::Solid("#AABBCCFF".into())));
    let Shape::Path { fill, color, .. } = &children[1] else {
        panic!()
    };
    assert!(color.is_none());
    assert_eq!(fill, &Some(Paint::Solid("#000000FF".into())));
}
#[test]
fn circles_ellipses_and_rounded_rectangles_lower_to_bounded_curves() {
    let scene=svg_scene(r#"<svg viewBox="0 0 100 100"><circle cx="20" cy="20" r="10"/><ellipse cx="50" cy="50" rx="20" ry="8"/><rect x="1" y="1" width="90" height="90" rx="12"/></svg>"#).unwrap();
    let Shape::Group { children, .. } = &scene[0] else {
        panic!()
    };
    assert_eq!(children.len(), 3);
    for s in children {
        let Shape::Path { commands, .. } = s else {
            panic!()
        };
        assert!(commands.iter().any(|c| matches!(c, P::Cubic { .. })));
        assert!(commands.len() <= 18);
    }
}
#[test]
fn gradients_use_true_geometry_bounds_and_source_space() {
    let scene=svg_scene(r##"<svg viewBox="0 0 100 100"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="red"/><stop offset="100%" stop-color="#00f" stop-opacity=".5"/></linearGradient></defs><rect x="10" y="20" width="40" height="80" fill="url(#g)"/></svg>"##).unwrap();
    let Shape::Group { children, .. } = &scene[0] else {
        panic!()
    };
    let Shape::Path {
        fill: Some(Paint::SvgLinear(g)),
        ..
    } = &children[0]
    else {
        panic!()
    };
    let at = |x, y| g.projection[0] * x + g.projection[1] * y + g.projection[2];
    assert!(at(10., 20.).abs() < 1e-10);
    assert!((at(50., 100.) - 1.).abs() < 1e-10);
    assert!((at(50., 20.) - 0.5).abs() < 1e-10);
    assert_eq!(g.to, "#0000FF80");
    let scene=svg_scene(r##"<svg viewBox="10 20 100 100"><defs><linearGradient id="g" gradientUnits="userSpaceOnUse" gradientTransform="translate(5)" x1="0" y1="0" x2="10" y2="0"><stop offset="0"/><stop offset="1" stop-color="white"/></linearGradient></defs><path d="M0 0H100V100Z" fill="url(#g)"/></svg>"##).unwrap();
    let Shape::Group { children, .. } = &scene[0] else {
        panic!()
    };
    let Shape::Path {
        fill: Some(Paint::SvgLinear(g)),
        ..
    } = &children[0]
    else {
        panic!()
    };
    assert_eq!(g.projection, [0.1, 0., -0.5]);
}
#[test]
fn svg_fit_metadata_immutability_and_hidden_bridge() {
    let source = r#"const icon=ui.svg('<svg viewBox="10 20 40 20"><rect x="10" y="20" width="40" height="20"/></svg>');
      if('__visualParseVector' in globalThis || !Object.isFrozen(icon) || !Object.isFrozen(icon.viewBox))throw Error('capability leak');
      app({title:'SVG',state:{},view:ui.canvas({id:'s',draw(g){g.svg(icon,{x:5,y:7,width:100,height:100});}})});"#;
    let scene = Runtime::from_bundle(source).unwrap().scene("s").unwrap();
    let Shape::Group { transform, .. } = &scene[0] else {
        panic!()
    };
    assert_eq!(*transform, [2.5, 0., 0., 2.5, -20., -18.]);
    let scene=svg_scene(r#"<svg viewBox="10 20 40 20" width="100" height="100" preserveAspectRatio="none"><path d="M10 20L50 40"/></svg>"#).unwrap();
    let Shape::Group { transform, .. } = &scene[0] else {
        panic!()
    };
    assert_eq!(*transform, [2.5, 0., 0., 5., -25., -100.]);
    assert!(
        Runtime::from_bundle(&drawing("g.svg({width:10,height:10,viewBox:[0,0,10,10]});"))
            .unwrap()
            .scene("s")
            .is_err()
    );
}
#[test]
fn rejects_active_external_unsupported_and_ambiguous_svg() {
    for xml in [
        "<!DOCTYPE svg><svg/>",
        "<!DOCTYPE svg [<!ENTITY x SYSTEM 'file:///etc/passwd'>]><svg>&x;</svg>",
        "<?xml-stylesheet href='https://example.com/x'?><svg/>",
        "<svg xmlns='urn:other'/>",
        "<svg><script>alert(1)</script></svg>",
        "<svg onload='x()'/>",
        "<svg><foreignObject/></svg>",
        "<svg><image href='data:image/png;base64,AA'/></svg>",
        "<svg><use href='#x'/></svg>",
        "<svg><text>Hi</text></svg>",
        "<svg><style>path{fill:red}</style></svg>",
        "<svg style='fill:red'/>",
        "<svg><clipPath/></svg>",
        "<svg><g opacity='.5'/></svg>",
        "<svg><g fill='url(https://example.com/g)'/></svg>",
        "<svg><defs><image/></defs></svg>",
        "<svg><defs><linearGradient id='a' href='#b'/></defs></svg>",
        "<svg><path id='x'/><path id='x'/></svg>",
        "<svg><path><g/></path></svg>",
        "<svg><svg/></svg>",
        "<svg><circle r='-1'/></svg>",
        "<svg><path d='M0 0' fill='#ggg'/></svg>",
        "<svg stroke-linecap='round'/>",
        "<svg stroke-linejoin='bevel'/>",
        "<svg stroke-dasharray='2 2'/>",
        "<svg viewBox='0 0 0 10'/>",
        "<svg width='100%'/>",
        "<svg preserveAspectRatio='xMinYMin slice'/>",
        "<svg><stop/></svg>",
        "<svg><linearGradient/></svg>",
        "<svg overflow='hidden'/>",
        "<svg xmlns:x='urn:bad' x:fill='red'/>",
        "<svg><path d='M0 0' fill='url(#missing)'/></svg>",
    ] {
        assert!(svg_scene(xml).is_err(), "accepted {xml}");
    }
    for d in [
        "",
        "L0 0",
        "M0",
        "M0 0A1 1 0 2 0 4 5",
        "M0 0L1e999 0",
        "M1000000 0l1 0",
        "M0 0C0 0 0 0 0",
        "M0 0junk",
    ] {
        let app = Runtime::from_bundle(&drawing(&format!(
            "g.path(ui.path({}),{{fill:'#FFFFFF'}});",
            json!(d)
        )))
        .unwrap();
        assert!(app.scene("s").is_err(), "accepted {d}");
    }
}
#[test]
fn shared_scene_budget_counts_each_svg_instance_and_combined_nesting() {
    let declaration = "const a=ui.svg('<svg><rect width=\"2\" height=\"2\"/></svg>');";
    for draw in [
        "for(let i=0;i<33;i++)g.svg(a);",
        "function nest(n){if(n)g.group({},()=>nest(n-1));else g.svg(a);}nest(8);",
    ] {
        let app = Runtime::from_bundle(&(declaration.to_owned() + &drawing(draw))).unwrap();
        assert!(app.scene("s").is_err());
    }
    let xml = format!("<svg>{}</svg>", "<circle r='10'/>".repeat(9));
    assert!(svg_scene(&xml).is_err());
    let xml = format!(
        "<svg>{}<path d='M0 0'/>{}</svg>",
        "<g>".repeat(20),
        "</g>".repeat(20)
    );
    assert!(svg_scene(&xml).is_err());
    let app = Runtime::from_bundle(&drawing("for(let i=0;i<33;i++)ui.svg('<svg/>');")).unwrap();
    assert!(app.scene("s").is_err());
    let xml = format!("<svg><!--{}--></svg>", "x".repeat(16384));
    assert!(svg_scene(&xml).is_err());
    let xml = format!("<svg>{}</svg>", "<!--x-->".repeat(600));
    assert!(svg_scene(&xml).is_err());
}
#[test]
fn cached_imports_do_not_change_output_or_break_transaction_rollback() {
    let mut app=Runtime::from_bundle(r#"const icon=ui.svg('<svg><path d="M0 0H10V10Z"/></svg>');app({title:'Cache',state:{bad:false,x:0},view:ui.canvas({id:'s',draw(g,s){g.svg(icon,{x:s.x});if(s.bad)ui.svg('<svg><image/></svg>');}})});"#).unwrap();
    let first = app.scene("s").unwrap();
    app.restore_state(&json!({"bad":true,"x":10})).unwrap();
    assert!(app.scene("s").is_err());
    assert_eq!(app.last_scene("s").unwrap().as_ref(), first.as_ref());
    app.restore_state(&json!({"bad":false,"x":0})).unwrap();
    assert_eq!(app.scene("s").unwrap().as_ref(), first.as_ref());
}

#[test]
fn numerical_arc_regressions_never_panic_and_correct_tiny_nonzero_radii() {
    assert_eq!(path("M0 0 A1 1 0 0 1 1e-200 0").len(), 2);
    let corrected = path("M0 0 A0.000001 0.000001 0 0 1 100 0");
    assert!(corrected.iter().any(|c| matches!(c, P::Cubic { .. })));
    assert!(
        corrected
            .iter()
            .any(|c| matches!(c,P::Cubic{to,..} if to[1].abs()>45.))
    );
    for exponent in [-300, -200, -100, -30, -6, 0, 3, 6] {
        for large in [0, 1] {
            for sweep in [0, 1] {
                let d = format!("M0 0 A1e{exponent} 1e{exponent} 30 {large} {sweep} 10 20");
                let source = drawing(&format!(
                    "g.path(ui.path({}),{{fill:'#FFFFFF'}});",
                    json!(d)
                ));
                let app = Runtime::from_bundle(&source).unwrap();
                let result = app.scene("s");
                if let Ok(scene) = result {
                    let Shape::Path { commands, .. } = &scene[0] else {
                        panic!()
                    };
                    assert!(commands.len() <= 9);
                }
            }
        }
    }
}
#[test]
fn percentage_user_gradients_and_degenerate_bbox_follow_svg_semantics() {
    let template = r##"<svg viewBox="10 20 100 100"><defs><linearGradient id="g" gradientUnits="userSpaceOnUse" x1="START" x2="END"><stop offset="0"/><stop offset="1" stop-color="white"/></linearGradient></defs><rect width="100" height="100" fill="url(#g)"/></svg>"##;
    let percent = svg_scene(&template.replace("START", "0%").replace("END", "100%")).unwrap();
    let numeric = svg_scene(&template.replace("START", "0").replace("END", "100")).unwrap();
    assert_eq!(*percent, *numeric);
    let scene=svg_scene(r##"<svg><defs><linearGradient id="g"><stop offset="0"/><stop offset="1" stop-color="white"/></linearGradient></defs><line x1="10" y1="20" x2="90" y2="20" fill="none" stroke="url(#g)" stroke-width="10"/></svg>"##).unwrap();
    let Shape::Group { children, .. } = &scene[0] else {
        panic!()
    };
    assert!(children.is_empty());
}

#[test]
fn svg_postcard_sample_controls_and_animation_stay_within_shared_limits() {
    let source = include_str!("../../pi_visual/assets/samples/svg-postcard.pi-visual.js");
    let mut app = Runtime::from_bundle(source).unwrap();
    for angle in [-30., -8., 0., 30.] {
        for zoom in [0.65, 1., 1.15] {
            app.restore_state(&json!({"angle":angle,"zoom":zoom,"playing":false}))
                .unwrap();
            let scene = app.scene("postcard").unwrap();
            assert!(!scene.is_empty());
        }
    }
    app.restore_state(&json!({"angle":28.,"zoom":1.,"playing":true}))
        .unwrap();
    for _ in 0..32 {
        app.tick(0.1).unwrap();
        assert!(!app.scene("postcard").unwrap().is_empty());
    }
}

#[test]
fn host_quotas_include_failed_requests_registration_and_utf8_bytes() {
    let app = Runtime::from_bundle(&drawing(
        "for(let i=0;i<32;i++){try{ui.svg('<svg><image/></svg>');}catch(e){}}ui.svg('<svg/>');",
    ))
    .unwrap();
    assert!(app.scene("s").is_err());
    let source =
        "for(let i=0;i<32;i++)ui.svg('<svg/>');".to_owned() + &drawing("ui.svg('<svg/>');");
    let app = Runtime::from_bundle(&source).unwrap();
    assert!(app.scene("s").is_err());
    let xml = format!("<svg><!--{}--></svg>", "界".repeat(6000));
    assert!(svg_scene(&xml).is_err());
}

#[test]
fn tiny_well_conditioned_svg_gradients_can_be_magnified() {
    let scene=svg_scene(r##"<svg viewBox="0 0 0.0000005 0.0000005"><defs><linearGradient id="g"><stop offset="0"/><stop offset="1" stop-color="white"/></linearGradient></defs><rect width="0.0000005" height="0.0000005" fill="url(#g)"/></svg>"##).unwrap();
    let Shape::Group { children, .. } = &scene[0] else {
        panic!()
    };
    let Shape::Path {
        fill: Some(Paint::SvgLinear(g)),
        ..
    } = &children[0]
    else {
        panic!()
    };
    assert!((g.projection[0] - 2e6).abs() < 0.1);
}
#[test]
fn guest_iterator_poisoning_cannot_capture_or_mutate_imported_commands() {
    let source = r##"const icon=ui.svg('<svg><path d="M0 0H10V10Z"/></svg>');
      const it=Object.getPrototypeOf([].values());let leaked=false;
      try {it.next=function(){leaked=true;return {done:true};};}catch(e){}
      if(!Object.isFrozen(it))throw Error('mutable iterator');
      app({title:'Sealed',state:{},view:ui.canvas({id:'s',draw(g){g.svg(icon);if(leaked)throw Error('iterator hook ran');}})});"##;
    let scene = Runtime::from_bundle(source).unwrap().scene("s").unwrap();
    let Shape::Group { children, .. } = &scene[0] else {
        panic!()
    };
    let Shape::Path { fill, .. } = &children[0] else {
        panic!()
    };
    assert_eq!(fill, &Some(Paint::Solid("#000000FF".into())));
}
