//! Remembers the last painted frame so an identical one is not painted again (#86).

use super::taskbar::Rect;
use super::view::ItemView;

/// Everything the painted pixels and the strip's position depend on.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameKey {
    pub items: Vec<ItemView>,
    pub taskbar: Rect,
    pub dpi: u32,
    /// Width the layout may use, in pixels.
    pub available: i32,
    pub light: bool,
}

#[derive(Debug, Default)]
pub struct FrameCache {
    last: Option<FrameKey>,
}

impl FrameCache {
    /// Whether `key` is the frame already on screen.
    pub fn is_current(&self, key: &FrameKey) -> bool {
        self.last.as_ref() == Some(key)
    }

    /// Records a frame that was painted successfully.
    pub fn painted(&mut self, key: FrameKey) {
        self.last = Some(key);
    }

    /// Forces the next frame to paint (hidden strip, failed paint, new taskbar or display).
    pub fn invalidate(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ItemId, Kind};

    fn key(cpu: f32) -> FrameKey {
        FrameKey {
            items: vec![ItemView {
                id: ItemId::Cpu,
                kind: Kind::Hardware,
                label: "CPU".into(),
                value: Some(cpu),
                dim: false,
                show_remaining: false,
                badge: None,
            }],
            taskbar: Rect {
                left: 0,
                top: 1032,
                right: 1920,
                bottom: 1080,
            },
            dpi: 96,
            available: 700,
            light: false,
        }
    }

    #[test]
    fn nothing_is_current_before_the_first_paint() {
        assert!(!FrameCache::default().is_current(&key(10.0)));
    }

    #[test]
    fn the_painted_frame_is_current_until_anything_changes() {
        let mut cache = FrameCache::default();
        cache.painted(key(10.0));
        assert!(cache.is_current(&key(10.0)));
        // Same text ("10%") but a different bar width: still a new frame.
        assert!(!cache.is_current(&key(10.4)));
        for changed in [
            FrameKey {
                dpi: 120,
                ..key(10.0)
            },
            FrameKey {
                available: 500,
                ..key(10.0)
            },
            FrameKey {
                light: true,
                ..key(10.0)
            },
            FrameKey {
                taskbar: Rect::default(),
                ..key(10.0)
            },
        ] {
            assert!(!cache.is_current(&changed), "{changed:?}");
        }
    }

    #[test]
    fn invalidate_forces_a_repaint() {
        let mut cache = FrameCache::default();
        cache.painted(key(10.0));
        cache.invalidate();
        assert!(!cache.is_current(&key(10.0)));
    }
}
