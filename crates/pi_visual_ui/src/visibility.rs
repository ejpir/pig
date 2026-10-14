//! A mounted-card lease, including cards with animation but no canvas.
use gpui::{App, IntoElement, RenderOnce, SharedString, Window, canvas, prelude::*};
use std::{cell::Cell, rc::Rc};

struct Presence(Rc<Cell<bool>>);
type Paint = Box<dyn FnOnce(&Rc<Cell<bool>>, &mut Window, &mut App)>;
#[derive(IntoElement)]
pub(crate) struct Marker {
    pub id: SharedString,
    pub paint: Paint,
}
impl RenderOnce for Marker {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let presence = window.use_keyed_state(self.id, cx, |_, cx| {
            cx.on_release(|state: &mut Presence, _| state.0.set(false))
                .detach();
            Presence(Rc::new(Cell::new(false)))
        });
        presence.read(cx).0.set(false);
        canvas(
            |bounds, _, _| bounds,
            move |_, bounds, window, cx| {
                let painted = presence.read(cx).0.clone();
                painted.set(bounds.intersects(&window.content_mask().bounds));
                (self.paint)(&painted, window, cx);
            },
        )
        .absolute()
        .inset_0()
    }
}
