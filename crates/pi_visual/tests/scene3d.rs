use pi_visual::{Runtime, Shape};

fn scene(source: &str) -> anyhow::Result<std::sync::Arc<Vec<Shape>>> {
    let runtime = Runtime::from_bundle(source)?;
    runtime.scene("scene")
}

#[test]
fn scripts_send_mesh_descriptors_and_transforms_not_triangles() {
    let scene = scene(
        "const ball=ui.sphere(1,{segments:32});
        app({title:'3D',state:{t:10},view:ui.canvas({id:'scene',viewBox:[0,0,100,100],draw(g,s){
            g.scene3d({camera:{distance:6,yaw:s.t}},d=>{
                d.mesh(ball,{position:[1,0,0],rotate:[0,s.t,0],scale:2,color:'#FF0000'});
                d.line([[0,0,0],[1,1,1]],{color:'#00FF00',width:2});
            });
        }})});",
    )
    .unwrap();
    let [
        Shape::Scene3d {
            camera,
            objects,
            viewport,
            ..
        },
    ] = scene.as_slice()
    else {
        panic!("{scene:?}")
    };
    assert_eq!(*viewport, None);
    assert_eq!(camera.yaw, 10.);
    assert_eq!(objects.len(), 2);
    let json = serde_json::to_string(scene.as_slice()).unwrap();
    assert!(json.len() < 600, "{json}");
}

#[test]
fn only_declared_meshes_and_bounded_scenes_are_accepted() {
    let draw = |body: &str| {
        scene(&format!(
            "const m=ui.box(1,1,1);app({{title:'3D',state:{{}},view:ui.canvas({{id:'scene',draw(g,s){{{body}}}}})}});"
        ))
    };
    assert!(draw("g.scene3d({},d=>d.mesh(m))").is_ok());
    // Forged descriptors, bad cameras and oversized scenes fail explicitly.
    assert!(draw("g.scene3d({},d=>d.mesh({kind:'box',size:[1,1,1]}))").is_err());
    assert!(draw("g.scene3d({camera:{pitch:90}},d=>d.mesh(m))").is_err());
    assert!(draw("g.scene3d({},d=>{for(let i=0;i<65;i++)d.mesh(m);})").is_err());
    assert!(
        draw(
            "const s=ui.sphere(1,{segments:64});g.scene3d({},d=>{d.mesh(s);d.mesh(s);d.mesh(s);})"
        )
        .is_err()
    );
    assert!(draw("for(let i=0;i<5;i++)g.scene3d({},d=>d.mesh(m));").is_err());
    assert!(draw("g.scene3d({},d=>d.line([[0,0,0]]))").is_err());
}

#[test]
fn custom_meshes_are_validated_before_they_reach_the_host() {
    let source = |mesh: &str| {
        format!(
            "const m=ui.mesh({mesh});app({{title:'3D',state:{{}},view:ui.canvas({{id:'scene',draw(g,s){{g.scene3d({{}},d=>d.mesh(m));}}}})}});"
        )
    };
    assert!(
        scene(&source(
            "{vertices:[[0,0,0],[1,0,0],[0,1,0]],faces:[[0,1,2]]}"
        ))
        .is_ok()
    );
    assert!(Runtime::from_bundle(&source("{vertices:[[0,0,0]],faces:[[0,1,2]]}")).is_err());
    assert!(Runtime::from_bundle(&source("{vertices:[[0,0,1e9]],faces:[[0,0,0]]}")).is_err());
}

#[test]
fn every_option_the_guide_documents_is_accepted() {
    let scene = scene(
        "const shapes=[ui.box(1,1,1),ui.sphere(1,{segments:24}),ui.cylinder(1,2,{segments:16}),
            ui.cone(1,2,{segments:16}),ui.torus(2,0.5,{segments:24}),
            ui.mesh({vertices:[[0,0,0],[1,0,0],[0,1,0]],faces:[[0,1,2]]})];
        app({title:'3D',state:{},view:ui.canvas({id:'scene',viewBox:[0,0,200,100],draw(g,s){
            g.scene3d({x:0,y:0,width:100,height:100,camera:{fov:50,distance:5,yaw:30,pitch:20,target:[0,0,0]},light:[1,1,1]},d=>{
                shapes.forEach((m,i)=>d.mesh(m,{position:[i,0,0],rotate:[10,20,30],scale:[1,2,1],color:'#4D9ABF80'}));
                d.line([[0,0,0],[1,1,1],[2,0,0]],{color:'#F1BE58',width:2});
            });
        }})});",
    )
    .unwrap();
    let [
        Shape::Scene3d {
            viewport,
            light,
            objects,
            ..
        },
    ] = scene.as_slice()
    else {
        panic!("{scene:?}")
    };
    assert_eq!(*viewport, Some([0., 0., 100., 100.]));
    assert_eq!(*light, Some([1., 1., 1.]));
    assert_eq!(objects.len(), 7);
}

#[test]
fn the_solids_sample_runs_and_stays_well_inside_the_limits() {
    let source = include_str!("../assets/samples/solids-3d.pi-visual.js");
    let runtime = Runtime::from_bundle(source).unwrap();
    let draw = match &runtime.definition.view {
        pi_visual::Node::Column { children, .. } => children
            .iter()
            .find_map(|n| match n {
                pi_visual::Node::Canvas { draw, .. } => Some(draw.clone()),
                _ => None,
            })
            .unwrap(),
        _ => panic!(),
    };
    let scene = runtime.scene(&draw).unwrap();
    let [Shape::Scene3d { objects, .. }] = scene.as_slice() else {
        panic!()
    };
    let triangles: usize = objects
        .iter()
        .map(|o| match o {
            pi_visual::scene3d::Object::Mesh { mesh, .. } => mesh.triangles(),
            _ => 0,
        })
        .sum();
    assert!(triangles < 4096, "{triangles}");
}
