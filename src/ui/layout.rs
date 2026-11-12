//! Fits strip items into the free taskbar space (spec §6.7).

use crate::model::Kind;

/// Item width measured by the renderer, in physical pixels: the wider of the label and
/// the value with its bar, plus the account badge.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemMetrics {
    pub width: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// Indices into the input slice, in display order.
    pub visible: Vec<usize>,
    /// Input index of the last hardware item, when limit items follow it.
    pub separator_after: Option<usize>,
    /// Items were dropped.
    pub overflow: bool,
    /// Position in `visible` before which the ellipsis is drawn, where the dropped items
    /// were: after the visible hardware while only hardware is hidden, else at the end.
    pub ellipsis_at: Option<usize>,
    pub width: i32,
}

/// Width of the `shown` items (input indices, in display order) and the separator position.
fn measure(
    items: &[(Kind, ItemMetrics)],
    shown: &[usize],
    gap: i32,
    sep_w: i32,
) -> (i32, Option<usize>) {
    let mut width: i32 = shown.iter().map(|&i| items[i].1.width).sum();
    width += gap * (shown.len() as i32 - 1).max(0);
    let last_hw = shown
        .iter()
        .rev()
        .copied()
        .find(|&i| items[i].0 == Kind::Hardware);
    let has_limits = shown.iter().any(|&i| items[i].0 != Kind::Hardware);
    let separator_after = last_hw.filter(|_| has_limits);
    if separator_after.is_some() {
        width += sep_w + gap;
    }
    (width, separator_after)
}

/// Order in which items are dropped when space runs out: hardware first (from its last
/// item), then the limits from the end. Usage limits are the reason CCT exists.
fn drop_order(items: &[(Kind, ItemMetrics)]) -> Vec<usize> {
    let (hardware, limits): (Vec<usize>, Vec<usize>) =
        (0..items.len()).partition(|&i| items[i].0 == Kind::Hardware);
    hardware
        .into_iter()
        .rev()
        .chain(limits.into_iter().rev())
        .collect()
}

/// Everything when it fits, else drops items (hardware first, see [`drop_order`]) and adds
/// an ellipsis. Labels always stay. Returns width 0 when nothing fits.
pub fn layout(
    items: &[(Kind, ItemMetrics)],
    available: i32,
    gap: i32,
    sep_w: i32,
    ellipsis_w: i32,
) -> Layout {
    let n = items.len();
    let mut visible: Vec<usize> = (0..n).collect();
    let (width, separator_after) = measure(items, &visible, gap, sep_w);
    if width <= available {
        return Layout {
            visible,
            separator_after,
            overflow: false,
            ellipsis_at: None,
            width,
        };
    }
    let mut limits_dropped = false;
    for dropped in drop_order(items).into_iter().take(n.saturating_sub(1)) {
        limits_dropped |= items[dropped].0 != Kind::Hardware;
        visible.retain(|&i| i != dropped);
        let (width, separator_after) = measure(items, &visible, gap, sep_w);
        let width = width + gap + ellipsis_w;
        if width <= available {
            let shown_hw = visible
                .iter()
                .filter(|&&i| items[i].0 == Kind::Hardware)
                .count();
            return Layout {
                ellipsis_at: Some(if limits_dropped {
                    visible.len()
                } else {
                    shown_hw
                }),
                visible,
                separator_after,
                overflow: true,
                width,
            };
        }
    }
    Layout {
        visible: Vec::new(),
        separator_after: None,
        overflow: n > 0,
        ellipsis_at: None,
        width: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Kind;

    const GAP: i32 = 10;
    const SEP: i32 = 1;
    const ELL: i32 = 12;

    fn items() -> Vec<(Kind, ItemMetrics)> {
        let hw = ItemMetrics { width: 40 };
        let cl = ItemMetrics { width: 50 };
        vec![
            (Kind::Hardware, hw),
            (Kind::Hardware, hw),
            (Kind::Hardware, hw),
            (Kind::Claude, cl),
            (Kind::Claude, cl),
        ]
    }

    #[test]
    fn fits_everything() {
        let l = layout(&items(), 300, GAP, SEP, ELL);
        assert_eq!(l.visible, vec![0, 1, 2, 3, 4]);
        assert_eq!(l.separator_after, Some(2));
        assert!(!l.overflow);
        assert_eq!(l.width, 271);
    }

    #[test]
    fn drops_hardware_before_limits() {
        // Hardware goes first, from its last item: quotas are why CCT exists. Labels stay,
        // so the strip never trades every label for one more hardware item.
        let l = layout(&items(), 250, GAP, SEP, ELL);
        assert_eq!(l.visible, vec![0, 1, 3, 4]);
        assert!(l.overflow);
        assert_eq!(l.separator_after, Some(1));
        assert_eq!(l.width, 243);
    }

    #[test]
    fn drops_limits_from_end_once_hardware_is_gone() {
        let l = layout(&items(), 80, GAP, SEP, ELL);
        assert_eq!(l.visible, vec![3]);
        assert!(l.overflow);
        assert_eq!(l.separator_after, None);
        assert_eq!(l.width, 72);
    }

    #[test]
    fn ellipsis_sits_where_items_were_dropped() {
        // Hidden hardware: right after the visible hardware, before the separator.
        assert_eq!(layout(&items(), 250, GAP, SEP, ELL).ellipsis_at, Some(2));
        // All hardware hidden, limits whole: in front of the limits.
        assert_eq!(layout(&items(), 150, GAP, SEP, ELL).ellipsis_at, Some(0));
        // Limits hidden too: at the end.
        assert_eq!(layout(&items(), 80, GAP, SEP, ELL).ellipsis_at, Some(1));
        // Nothing hidden: no ellipsis.
        assert_eq!(layout(&items(), 300, GAP, SEP, ELL).ellipsis_at, None);
    }

    #[test]
    fn separator_only_between_groups() {
        let hw_only: Vec<_> = items().into_iter().take(3).collect();
        assert_eq!(layout(&hw_only, 500, GAP, SEP, ELL).separator_after, None);
        let limits_only: Vec<_> = items().into_iter().skip(3).collect();
        assert_eq!(
            layout(&limits_only, 500, GAP, SEP, ELL).separator_after,
            None
        );
    }

    #[test]
    fn empty_items_zero_width() {
        let l = layout(&[], 500, GAP, SEP, ELL);
        assert!(l.visible.is_empty());
        assert_eq!(l.width, 0);
        assert!(!l.overflow);
    }

    #[test]
    fn nothing_fits_hides_strip() {
        let l = layout(&items(), 5, GAP, SEP, ELL);
        assert!(l.visible.is_empty());
        assert_eq!(l.width, 0);
    }
}
