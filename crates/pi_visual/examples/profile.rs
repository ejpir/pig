//! Host-only repeatable invocation benchmark. Not a presentation/FPS benchmark.
use anyhow::{Context, Result};
use pi_visual::{Node, Runtime};
use std::{collections::BTreeMap, time::Instant};

fn bindings(node: &Node, texts: &mut Vec<String>, draws: &mut Vec<String>) {
    match node {
        Node::Column { children, .. } | Node::Row { children, .. } => {
            for child in children {
                bindings(child, texts, draws);
            }
        }
        Node::Text { bind: Some(id), .. } => texts.push(id.clone()),
        Node::Canvas { draw, .. } => draws.push(draw.clone()),
        _ => {}
    }
}
fn summary(mut values: Vec<f64>) -> (f64, f64) {
    values.sort_by(f64::total_cmp);
    let last = values.len() - 1;
    (values[last / 2], values[last * 95 / 100])
}
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .context("Usage: profile app.js [slider-field] [iterations]")?;
    let field = args.next().unwrap_or_else(|| "ring".into());
    let iterations: usize = args.next().unwrap_or_else(|| "240".into()).parse()?;
    anyhow::ensure!(
        (10..=10_000).contains(&iterations),
        "Use 10..10000 iterations"
    );
    let source = std::fs::read_to_string(path)?;
    let mut runtime = Runtime::from_bundle(&source)?;
    let value = runtime
        .number(&field)
        .context("Unknown numeric slider field")?;
    let mut texts = Vec::new();
    let mut draws = Vec::new();
    bindings(&runtime.definition.view, &mut texts, &mut draws);
    runtime.enable_profiling(true);
    let mut times: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for i in 0..iterations + 10 {
        let start = Instant::now();
        runtime.slide(&field, value + (i % 2) as f64)?;
        for id in &texts {
            runtime.text(id);
        }
        for id in &draws {
            runtime.scene(id)?;
        }
        let total = start.elapsed().as_secs_f64() * 1000.;
        let batch = runtime.take_profile();
        if i < 10 {
            continue;
        }
        times.entry("Total".into()).or_default().push(total);
        for (stage, elapsed) in batch.times {
            times
                .entry(format!("{stage:?}"))
                .or_default()
                .push(elapsed.as_secs_f64() * 1000.);
        }
    }
    println!(
        "Host CPU invocation wall times: {iterations} slider/state/text/scene cycles (not display FPS)"
    );
    for (stage, values) in times {
        let count = values.len();
        let (median, p95) = summary(values);
        println!("{stage:16} median {median:7.3} ms  p95 {p95:7.3} ms  samples {count}");
    }
    Ok(())
}
