use anyhow::Result;
use gpui::{Bounds, DisplayId, Pixels, PlatformDisplay, Point, Size};
use std::cell::Cell;

/// The screen the activity is on. Its bounds follow the activity's window,
/// which fills the screen.
#[derive(Debug, Default)]
pub(crate) struct AndroidDisplay {
    size: Cell<Size<Pixels>>,
}

impl AndroidDisplay {
    pub fn set_size(&self, size: Size<Pixels>) {
        self.size.set(size);
    }
}

impl PlatformDisplay for AndroidDisplay {
    fn id(&self) -> DisplayId {
        DisplayId::new(0)
    }

    fn uuid(&self) -> Result<uuid::Uuid> {
        // One built-in screen; stable across launches.
        Ok(uuid::Uuid::from_u128(
            0x7069_616e_6472_6f69_6400_0000_0000_0001,
        ))
    }

    fn bounds(&self) -> Bounds<Pixels> {
        Bounds::new(Point::default(), self.size.get())
    }
}
