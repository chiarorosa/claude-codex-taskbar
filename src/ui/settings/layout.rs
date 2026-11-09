//! Settings page layout: title, captions, cards of rows, chips. Pure: text is measured through
//! `Measure` (DIPs), and every rect comes out in physical pixels for one scale.

pub const WIDTH: f32 = 460.0;
pub const PAD: f32 = 16.0;
pub const TITLE_GAP: f32 = 12.0;
pub const CAPTION_TOP: f32 = 14.0;
pub const CAPTION_BOTTOM: f32 = 6.0;
pub const CARD_PAD_X: f32 = 14.0;
pub const CARD_PAD_Y: f32 = 10.0;
pub const CARD_GAP: f32 = 3.0;
pub const CARD_RADIUS: f32 = 6.0;
pub const CHIP_GAP: f32 = 6.0;
pub const CHIP_PAD_X: f32 = 11.0;
pub const CHIP_PAD_Y: f32 = 4.0;
pub const SWITCH_W: f32 = 40.0;
pub const SWITCH_H: f32 = 20.0;
pub const TILE: f32 = 24.0;
pub const FIELD_W: f32 = 64.0;
pub const FIELD_H: f32 = 28.0;
pub const INDENT: f32 = 34.0;
/// Space between a row's text and its control, and between a state text and its switch.
const GAP: f32 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextRole {
    Title,
    Caption,
    Body,
    Secondary,
}

/// Which palette color a text run uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Text,
    Sub,
    Success,
    Warning,
}

pub trait Measure {
    /// Single-line width in DIPs.
    fn width(&self, text: &str, role: TextRole) -> f32;
    /// Wrapped height in DIPs.
    fn height(&self, text: &str, role: TextRole, max_width: f32) -> f32;
}

#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Switch {
        id: usize,
        title: String,
        sub: Option<(String, Ink)>,
        tile: Option<(char, u32)>,
        indent: bool,
        state_text: String,
    },
    Chips {
        label: String,
        chips: Vec<(usize, String)>,
    },
    Number {
        id: usize,
        title: String,
        sub: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Hint(String),
    Caption(String),
    Card(Vec<Row>),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Px {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Text {
    pub rect: Px,
    pub role: TextRole,
    pub ink: Ink,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub width: i32,
    pub height: i32,
    pub cards: Vec<Px>,
    pub hints: Vec<Px>,
    pub dividers: Vec<Px>,
    pub tiles: Vec<(Px, char, u32)>,
    pub texts: Vec<Text>,
    pub controls: Vec<(usize, Px)>,
}

/// Collects rects in DIPs and converts each one once.
struct Out<'a> {
    page: Page,
    scale: f32,
    m: &'a dyn Measure,
}

impl Out<'_> {
    fn px(&self, x: f32, y: f32, w: f32, h: f32) -> Px {
        let s = |v: f32| (v * self.scale).round() as i32;
        Px {
            x: s(x),
            y: s(y),
            w: s(w),
            h: s(h),
        }
    }

    fn text(&mut self, text: &str, role: TextRole, ink: Ink, x: f32, y: f32, w: f32) -> f32 {
        let h = self.m.height(text, role, w);
        let rect = self.px(x, y, w, h);
        self.page.texts.push(Text {
            rect,
            role,
            ink,
            text: text.to_string(),
        });
        h
    }

    /// Lays out one row starting at `top`; returns its height.
    fn row(&mut self, row: &Row, left: f32, right: f32, top: f32) -> f32 {
        match row {
            Row::Switch {
                id,
                title,
                sub,
                tile,
                indent,
                state_text,
            } => {
                let control_x = right - SWITCH_W;
                // Slack keeps DirectWrite from wrapping text that exactly fills its rect.
                let state_w = self.m.width(state_text, TextRole::Body).ceil() + 2.0;
                let state_x = control_x - GAP - state_w;
                let text_x = left
                    + if tile.is_some() || *indent {
                        INDENT
                    } else {
                        0.0
                    };
                let sub = sub.as_ref().map(|(t, ink)| (t.as_str(), *ink));
                let row_h = self.text_row(title, sub, text_x, state_x - GAP, top, SWITCH_H);
                let center = top + row_h / 2.0;
                let rect = self.px(control_x, center - SWITCH_H / 2.0, SWITCH_W, SWITCH_H);
                self.page.controls.push((*id, rect));
                let state_h = self.m.height(state_text, TextRole::Body, f32::MAX);
                self.text(
                    state_text,
                    TextRole::Body,
                    Ink::Sub,
                    state_x,
                    center - state_h / 2.0,
                    state_w,
                );
                if let Some((letter, color)) = tile {
                    let rect = self.px(left, center - TILE / 2.0, TILE, TILE);
                    self.page.tiles.push((rect, *letter, *color));
                }
                row_h
            }
            Row::Number { id, title, sub } => {
                let control_x = right - FIELD_W;
                let sub = Some((sub.as_str(), Ink::Sub));
                let row_h = self.text_row(title, sub, left, control_x - GAP, top, FIELD_H);
                let rect = self.px(control_x, top + (row_h - FIELD_H) / 2.0, FIELD_W, FIELD_H);
                self.page.controls.push((*id, rect));
                row_h
            }
            Row::Chips { label, chips } => {
                let mut y = top + CARD_PAD_Y;
                y += self.text(label, TextRole::Secondary, Ink::Sub, left, y, right - left);
                y += CHIP_GAP;
                let (mut x, mut bottom) = (left, y);
                for (id, text) in chips {
                    let line = self.m.height(text, TextRole::Body, f32::MAX);
                    let h = line + 2.0 * CHIP_PAD_Y;
                    let w =
                        (self.m.width(text, TextRole::Body) + 2.0 * CHIP_PAD_X).min(right - left);
                    if x > left && x + w > right {
                        x = left;
                        y = bottom + CHIP_GAP;
                    }
                    let rect = self.px(x, y, w, h);
                    self.page.controls.push((*id, rect));
                    x += w + CHIP_GAP;
                    bottom = bottom.max(y + h);
                }
                bottom + CARD_PAD_Y - top
            }
        }
    }

    /// Title and optional secondary line, vertically centered against a control of
    /// `control_h`; returns the row height.
    fn text_row(
        &mut self,
        title: &str,
        sub: Option<(&str, Ink)>,
        x: f32,
        right: f32,
        top: f32,
        control_h: f32,
    ) -> f32 {
        let w = (right - x).max(1.0);
        let title_h = self.m.height(title, TextRole::Body, w);
        let sub_h = sub.map_or(0.0, |(t, _)| self.m.height(t, TextRole::Secondary, w));
        let text_h = title_h + sub_h;
        let row_h = text_h.max(control_h) + 2.0 * CARD_PAD_Y;
        let mut y = top + (row_h - text_h) / 2.0;
        y += self.text(title, TextRole::Body, Ink::Text, x, y, w);
        if let Some((t, ink)) = sub {
            self.text(t, TextRole::Secondary, ink, x, y, w);
        }
        row_h
    }
}

pub fn layout(title: &str, blocks: &[Block], scale: f32, m: &dyn Measure) -> Page {
    let mut out = Out {
        page: Page::default(),
        scale,
        m,
    };
    let (x, w) = (PAD, WIDTH - 2.0 * PAD);
    let mut y = PAD;
    y += out.text(title, TextRole::Title, Ink::Text, x, y, w) + TITLE_GAP;
    for block in blocks {
        match block {
            Block::Hint(text) => {
                let text_w = w - 2.0 * CARD_PAD_X;
                let h = m.height(text, TextRole::Body, text_w) + 2.0 * CARD_PAD_Y;
                let rect = out.px(x, y, w, h);
                out.page.hints.push(rect);
                out.text(
                    text,
                    TextRole::Body,
                    Ink::Text,
                    x + CARD_PAD_X,
                    y + CARD_PAD_Y,
                    text_w,
                );
                y += h + CARD_GAP;
            }
            Block::Caption(text) => {
                y += CAPTION_TOP;
                y += out.text(text, TextRole::Caption, Ink::Text, x + 2.0, y, w) + CAPTION_BOTTOM;
            }
            Block::Card(rows) => {
                let top = y;
                for (i, row) in rows.iter().enumerate() {
                    if i > 0 {
                        let divider = out.px(x, y, w, 1.0);
                        out.page.dividers.push(divider);
                        y += 1.0;
                    }
                    y += out.row(row, x + CARD_PAD_X, x + w - CARD_PAD_X, y);
                }
                let card = out.px(x, top, w, y - top);
                out.page.cards.push(card);
                y += CARD_GAP;
            }
        }
    }
    out.page.width = (WIDTH * scale).round() as i32;
    out.page.height = ((y + PAD) * scale).round() as i32;
    out.page
}

/// Top-left corner that centers a window of `size` on the strip's monitor, or on the primary one.
pub fn center_on(monitor: Option<Px>, primary: Px, size: (i32, i32)) -> (i32, i32) {
    let m = monitor.unwrap_or(primary);
    (m.x + (m.w - size.0) / 2, m.y + (m.h - size.1) / 2)
}

/// The window rect: centered in the work area, never taller than it.
pub fn fit(work: Px, size: (i32, i32)) -> Px {
    let h = size.1.min(work.h);
    Px {
        x: work.x + (work.w - size.0) / 2,
        y: work.y + (work.h - h) / 2,
        w: size.0,
        h,
    }
}

/// A scroll offset kept between the top and the last full view of the content.
pub fn clamp_scroll(offset: i32, content_h: i32, view_h: i32) -> i32 {
    offset.clamp(0, (content_h - view_h).max(0))
}

/// The scroll offset that shows `item` (content coordinates) with `margin` around it.
pub fn reveal(offset: i32, item: Px, view_h: i32, margin: i32) -> i32 {
    if item.y - margin < offset {
        item.y - margin
    } else if item.y + item.h + margin > offset + view_h {
        item.y + item.h + margin - view_h
    } else {
        offset
    }
}

impl Page {
    /// The same page moved vertically by `dy` pixels (for drawing a scrolled view).
    pub fn shifted(&self, dy: i32) -> Page {
        let mv = |r: &Px| Px { y: r.y + dy, ..*r };
        Page {
            width: self.width,
            height: self.height,
            cards: self.cards.iter().map(mv).collect(),
            hints: self.hints.iter().map(mv).collect(),
            dividers: self.dividers.iter().map(mv).collect(),
            tiles: self.tiles.iter().map(|(r, c, k)| (mv(r), *c, *k)).collect(),
            texts: self
                .texts
                .iter()
                .map(|t| Text {
                    rect: mv(&t.rect),
                    ..t.clone()
                })
                .collect(),
            controls: self.controls.iter().map(|(i, r)| (*i, mv(r))).collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 7 DIPs per character, 18 DIPs per wrapped line.
    struct Fake;

    impl Measure for Fake {
        fn width(&self, text: &str, _role: TextRole) -> f32 {
            text.chars().count() as f32 * 7.0
        }
        fn height(&self, text: &str, role: TextRole, max_width: f32) -> f32 {
            18.0 * (self.width(text, role) / max_width).ceil()
        }
    }

    fn switch(id: usize, title: &str) -> Row {
        Row::Switch {
            id,
            title: title.into(),
            sub: Some(("✓ Login found".into(), Ink::Success)),
            tile: None,
            indent: false,
            state_text: "On".into(),
        }
    }

    /// One card with two switch rows.
    fn sample_blocks() -> Vec<Block> {
        vec![Block::Card(vec![
            switch(1, "Claude Code"),
            switch(2, "Codex"),
        ])]
    }

    #[test]
    fn chips_wrap_inside_the_card() {
        let chips: Vec<_> = (0..12).map(|i| (i, format!("Disco {i} (D:)"))).collect();
        let p = layout(
            "Configurações",
            &[Block::Card(vec![Row::Chips {
                label: "Hardware".into(),
                chips,
            }])],
            1.0,
            &Fake,
        );
        let card = &p.cards[0];
        assert!(
            p.controls
                .iter()
                .all(|(_, r)| r.x + r.w <= card.x + card.w - 14)
        );
        let rows: std::collections::BTreeSet<_> = p.controls.iter().map(|(_, r)| r.y).collect();
        assert!(rows.len() > 1, "wrapped");
    }

    #[test]
    fn scale_multiplies_every_rect() {
        let blocks = sample_blocks();
        let a = layout("T", &blocks, 1.0, &Fake);
        let b = layout("T", &blocks, 1.5, &Fake);
        assert_eq!(b.width, (460.0f32 * 1.5).round() as i32);
        for ((_, ra), (_, rb)) in a.controls.iter().zip(&b.controls) {
            assert!((rb.x - (ra.x as f32 * 1.5) as i32).abs() <= 1);
            assert!((rb.w - (ra.w as f32 * 1.5) as i32).abs() <= 1);
        }
    }

    #[test]
    fn rows_in_a_card_get_dividers_and_switches_sit_right() {
        let p = layout("T", &sample_blocks(), 1.0, &Fake);
        assert_eq!(p.dividers.len(), 1);
        let (_, sw) = p.controls[0];
        assert_eq!((sw.w, sw.h), (40, 20));
        assert_eq!(sw.x + sw.w, p.cards[0].x + p.cards[0].w - 14);
    }

    #[test]
    fn single_line_texts_get_room_so_they_never_wrap() {
        // A rect exactly as wide as the measured text makes DirectWrite wrap its last letter.
        let p = layout("T", &sample_blocks(), 1.0, &Fake);
        let on = p.texts.iter().find(|t| t.text == "On").expect("state text");
        assert!(on.rect.w > (Fake.width("On", TextRole::Body)) as i32);
    }

    #[test]
    fn hint_block_is_optional_and_grows_with_text() {
        let without = layout("T", &sample_blocks(), 1.0, &Fake).height;
        let mut with = vec![Block::Hint("x".repeat(200))];
        with.extend(sample_blocks());
        let p = layout("T", &with, 1.0, &Fake);
        assert_eq!(p.hints.len(), 1);
        assert!(p.height > without + 36);
    }

    #[test]
    fn window_fits_the_work_area() {
        // 1366×768 with a 40 px taskbar: a 780 px tall window is clamped and pinned inside.
        let work = Px {
            x: 0,
            y: 0,
            w: 1366,
            h: 728,
        };
        assert_eq!(
            fit(work, (476, 780)),
            Px {
                x: 445,
                y: 0,
                w: 476,
                h: 728
            }
        );
        assert_eq!(
            fit(work, (476, 600)),
            Px {
                x: 445,
                y: 64,
                w: 476,
                h: 600
            }
        );
    }

    #[test]
    fn scroll_stays_within_the_content() {
        assert_eq!(clamp_scroll(-5, 900, 700), 0);
        assert_eq!(clamp_scroll(150, 900, 700), 150);
        assert_eq!(clamp_scroll(400, 900, 700), 200);
        assert_eq!(clamp_scroll(50, 600, 700), 0, "no scrolling when it fits");
    }

    #[test]
    fn focus_scrolls_the_control_into_view() {
        let item = |y| Px {
            x: 0,
            y,
            w: 40,
            h: 20,
        };
        assert_eq!(reveal(0, item(100), 300, 8), 0, "already visible");
        assert_eq!(
            reveal(0, item(400), 300, 8),
            128,
            "below: bottom edge plus margin"
        );
        assert_eq!(
            reveal(200, item(50), 300, 8),
            42,
            "above: top edge minus margin"
        );
    }

    #[test]
    fn shifting_a_page_moves_every_rect() {
        let p = layout("T", &sample_blocks(), 1.0, &Fake);
        let s = p.shifted(-30);
        assert_eq!(s.cards[0].y, p.cards[0].y - 30);
        assert_eq!(s.controls[1].1.y, p.controls[1].1.y - 30);
        assert_eq!(s.texts[0].rect.y, p.texts[0].rect.y - 30);
        assert_eq!(s.dividers[0].y, p.dividers[0].y - 30);
        assert_eq!(s.height, p.height);
    }

    #[test]
    fn centers_on_strip_monitor_or_primary() {
        let primary = Px {
            x: 0,
            y: 0,
            w: 1920,
            h: 1040,
        };
        assert_eq!(center_on(None, primary, (460, 600)), (730, 220));
        let second = Px {
            x: 1920,
            y: 0,
            w: 2560,
            h: 1400,
        };
        assert_eq!(
            center_on(Some(second), primary, (460, 600)),
            (1920 + 1050, 400)
        );
    }
}
