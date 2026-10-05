//! Read-only telemetry for the separately packaged fixture APK.
use gpui::{IntoElement, Styled};

#[cfg(feature = "ui-test")]
#[derive(Default)]
pub(crate) struct State {
    pub fixture: String,
    pub scale: f32,
    pub viewport: [f32; 2],
    pub pointer: [f32; 2],
    pub bounds: std::collections::BTreeMap<String, [f32; 4]>,
}
#[cfg(feature = "ui-test")]
impl gpui::Global for State {}

pub(crate) fn probe(name: impl Into<String>) -> impl IntoElement {
    let name = name.into();
    gpui::canvas(
        move |bounds, _, cx| {
            #[cfg(feature = "ui-test")]
            if cx.has_global::<State>() {
                let state = cx.global_mut::<State>();
                state.bounds.insert(
                    name,
                    [
                        bounds.left().into(),
                        bounds.top().into(),
                        bounds.size.width.into(),
                        bounds.size.height.into(),
                    ],
                );
            }
            #[cfg(not(feature = "ui-test"))]
            let _ = (name, bounds, cx);
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}
