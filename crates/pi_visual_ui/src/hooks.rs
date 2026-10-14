//! Optional host-only instrumentation; never exposed to guest JavaScript.
use gpui::{App, Bounds, Global, IntoElement, Pixels, prelude::*};
use std::rc::Rc;
#[derive(Clone)]
pub struct Probe(pub ProbeCallback);
type ProbeCallback = Rc<dyn Fn(&str, Bounds<Pixels>, &mut App)>;
impl Global for Probe {}
pub fn probe(name: impl Into<String>) -> impl IntoElement {
    let name = name.into();
    let selector = name.clone();
    gpui::div()
        .absolute()
        .inset_0()
        .debug_selector(move || selector.clone())
        .child(
            gpui::canvas(
                move |bounds, _, cx| {
                    if let Some(probe) = cx.try_global::<Probe>().cloned() {
                        (probe.0)(&name, bounds, cx);
                    }
                },
                |_, _, _, _| {},
            )
            .size_full(),
        )
}
