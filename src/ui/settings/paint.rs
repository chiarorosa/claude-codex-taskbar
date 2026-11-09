//! Direct2D painting for the Settings page and its switches, chips and number fields.
//!
//! Everything is drawn in physical pixels (the render target runs at 96 DPI); text formats are
//! created at the window's scale, and `Measure` converts back to DIPs for the layout.

use super::layout::{Ink, Measure, Page, Px, TextRole};
use super::palette::Palette;
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_ROUNDED_RECT, D2D1CreateFactory, ID2D1DCRenderTarget, ID2D1Factory, ID2D1SolidColorBrush,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_METRICS, DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::HDC;
use windows::core::{Result, w};

/// Font sizes in DIPs.
const TITLE_SIZE: f32 = 20.0;
const CAPTION_SIZE: f32 = 12.0;
const BODY_SIZE: f32 = 13.0;
const SECONDARY_SIZE: f32 = 11.5;

/// Switch knob center and radius in pixels: on sits right and is larger, off sits left.
pub fn knob(r: Px, on: bool) -> (f32, f32, f32) {
    let k = r.h as f32 / 20.0;
    let (inset, radius) = (4.0 * k, if on { 6.0 } else { 5.0 } * k);
    let cy = r.y as f32 + r.h as f32 / 2.0;
    let cx = if on {
        (r.x + r.w) as f32 - inset - radius
    } else {
        r.x as f32 + inset + radius
    };
    (cx, cy, radius)
}

fn color(rgb: u32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((rgb >> 16) & 0xff) as f32 / 255.0,
        g: ((rgb >> 8) & 0xff) as f32 / 255.0,
        b: (rgb & 0xff) as f32 / 255.0,
        a: 1.0,
    }
}

fn rect_f(r: Px) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.x as f32,
        top: r.y as f32,
        right: (r.x + r.w) as f32,
        bottom: (r.y + r.h) as f32,
    }
}

/// Rounded rect inset by half a pixel so 1 px strokes land on whole pixels.
fn rounded(r: Px, radius: f32) -> D2D1_ROUNDED_RECT {
    let mut rect = rect_f(r);
    rect.left += 0.5;
    rect.top += 0.5;
    rect.right -= 0.5;
    rect.bottom -= 0.5;
    D2D1_ROUNDED_RECT {
        rect,
        radiusX: radius,
        radiusY: radius,
    }
}

struct Formats {
    title: IDWriteTextFormat,
    caption: IDWriteTextFormat,
    body: IDWriteTextFormat,
    secondary: IDWriteTextFormat,
    /// Body text centered both ways, for chips.
    chip: IDWriteTextFormat,
    /// Semibold centered letter, for provider tiles.
    tile: IDWriteTextFormat,
}

fn format(
    dw: &IDWriteFactory,
    size: f32,
    weight: DWRITE_FONT_WEIGHT,
    centered: bool,
) -> Result<IDWriteTextFormat> {
    // SAFETY: plain factory calls; DirectWrite falls back if the family is missing.
    unsafe {
        let f = dw.CreateTextFormat(
            w!("Segoe UI Variable Text"),
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!(""),
        )?;
        if centered {
            f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
        }
        Ok(f)
    }
}

impl Formats {
    fn new(dw: &IDWriteFactory, scale: f32) -> Result<Formats> {
        let (n, sb) = (DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD);
        Ok(Formats {
            title: format(dw, TITLE_SIZE * scale, sb, false)?,
            caption: format(dw, CAPTION_SIZE * scale, sb, false)?,
            body: format(dw, BODY_SIZE * scale, n, false)?,
            secondary: format(dw, SECONDARY_SIZE * scale, n, false)?,
            chip: format(dw, BODY_SIZE * scale, n, true)?,
            tile: format(dw, CAPTION_SIZE * scale, sb, true)?,
        })
    }

    fn get(&self, role: TextRole) -> &IDWriteTextFormat {
        match role {
            TextRole::Title => &self.title,
            TextRole::Caption => &self.caption,
            TextRole::Body => &self.body,
            TextRole::Secondary => &self.secondary,
        }
    }
}

pub struct Painter {
    dwrite: IDWriteFactory,
    target: ID2D1DCRenderTarget,
    formats: Formats,
    scale: f32,
}

impl Painter {
    pub fn new(dpi: u32) -> Result<Painter> {
        // SAFETY: factory creation has no preconditions.
        let d2d: ID2D1Factory =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        // SAFETY: as above.
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            // 96 DPI: one Direct2D unit is one pixel; scaling lives in the layout and fonts.
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        // SAFETY: valid factory and properties.
        let target = unsafe { d2d.CreateDCRenderTarget(&props)? };
        let scale = dpi as f32 / 96.0;
        let formats = Formats::new(&dwrite, scale)?;
        Ok(Painter {
            dwrite,
            target,
            formats,
            scale,
        })
    }

    pub fn set_dpi(&mut self, dpi: u32) -> Result<()> {
        self.scale = dpi as f32 / 96.0;
        self.formats = Formats::new(&self.dwrite, self.scale)?;
        Ok(())
    }

    /// Binds `hdc` over `client`, runs `f` between BeginDraw and EndDraw.
    fn draw(
        &self,
        hdc: HDC,
        client: Px,
        f: impl FnOnce(&ID2D1DCRenderTarget, &ID2D1SolidColorBrush) -> Result<()>,
    ) -> Result<()> {
        let rect = RECT {
            left: client.x,
            top: client.y,
            right: client.x + client.w,
            bottom: client.y + client.h,
        };
        // SAFETY: hdc is a valid paint DC for the duration of this call.
        unsafe {
            self.target.BindDC(hdc, &rect)?;
            self.target.BeginDraw();
            let brush = self.target.CreateSolidColorBrush(&color(0), None)?;
            let drawn = f(&self.target, &brush);
            let ended = self.target.EndDraw(None, None);
            drawn.and(ended)
        }
    }

    fn text(
        &self,
        rt: &ID2D1DCRenderTarget,
        brush: &ID2D1SolidColorBrush,
        text: &str,
        fmt: &IDWriteTextFormat,
        r: Px,
    ) {
        let wide: Vec<u16> = text.encode_utf16().collect();
        // SAFETY: valid target, format and brush.
        unsafe {
            rt.DrawText(
                &wide,
                fmt,
                &rect_f(r),
                brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    /// The window background: page fill, hint cards, cards, dividers, tiles, texts, and the
    /// focus ring around `focus`.
    pub fn page(
        &self,
        hdc: HDC,
        client: Px,
        page: &Page,
        pal: &Palette,
        focus: Option<Px>,
    ) -> Result<()> {
        let radius = super::layout::CARD_RADIUS * self.scale;
        self.draw(hdc, client, |rt, brush| {
            // SAFETY: drawing calls on a target between BeginDraw and EndDraw.
            unsafe {
                rt.Clear(Some(&color(pal.bg)));
                let set = |rgb: u32| brush.SetColor(&color(rgb));
                for &r in &page.hints {
                    set(pal.hint);
                    rt.FillRoundedRectangle(&rounded(r, radius), brush);
                    set(pal.hint_border);
                    rt.DrawRoundedRectangle(&rounded(r, radius), brush, 1.0, None);
                }
                for &r in &page.cards {
                    set(pal.card);
                    rt.FillRoundedRectangle(&rounded(r, radius), brush);
                    set(pal.border);
                    rt.DrawRoundedRectangle(&rounded(r, radius), brush, 1.0, None);
                }
                set(pal.border);
                for &r in &page.dividers {
                    rt.FillRectangle(&rect_f(r), brush);
                }
                for &(r, letter, rgb) in &page.tiles {
                    let tile = rounded(r, 6.0 * self.scale);
                    if pal.high_contrast {
                        // System colors only: an outlined tile in the text color.
                        set(pal.text);
                        rt.DrawRoundedRectangle(&tile, brush, 1.0, None);
                    } else {
                        set(rgb);
                        rt.FillRoundedRectangle(&tile, brush);
                        set(0xFFFFFF);
                    }
                    self.text(rt, brush, &letter.to_string(), &self.formats.tile, r);
                }
                for t in &page.texts {
                    set(match t.ink {
                        Ink::Text => pal.text,
                        Ink::Sub => pal.sub,
                        Ink::Success => pal.success,
                        Ink::Warning => pal.warning,
                    });
                    self.text(rt, brush, &t.text, self.formats.get(t.role), t.rect);
                }
                if let Some(f) = focus {
                    let gap = (3.0 * self.scale).round() as i32;
                    let ring = Px {
                        x: f.x - gap,
                        y: f.y - gap,
                        w: f.w + 2 * gap,
                        h: f.h + 2 * gap,
                    };
                    set(pal.text);
                    let r = (ring.h as f32 / 2.0).min(8.0 * self.scale);
                    rt.DrawRoundedRectangle(&rounded(ring, r), brush, 2.0 * self.scale, None);
                }
            }
            Ok(())
        })
    }

    /// One switch filling `client` (the control's own client area).
    pub fn switch(&self, hdc: HDC, client: Px, on: bool, pal: &Palette) -> Result<()> {
        self.draw(hdc, client, |rt, brush| {
            // SAFETY: drawing calls between BeginDraw and EndDraw.
            unsafe {
                rt.Clear(Some(&color(pal.card)));
                let pill = rounded(client, client.h as f32 / 2.0);
                if on {
                    brush.SetColor(&color(pal.accent));
                    rt.FillRoundedRectangle(&pill, brush);
                } else {
                    brush.SetColor(&color(pal.off_border));
                    rt.DrawRoundedRectangle(&pill, brush, self.scale.max(1.0), None);
                }
                let (cx, cy, r) = knob(client, on);
                brush.SetColor(&color(if on { pal.on_accent } else { pal.off_knob }));
                // A circle as a fully rounded square (no extra numerics types needed).
                rt.FillRoundedRectangle(
                    &D2D1_ROUNDED_RECT {
                        rect: D2D_RECT_F {
                            left: cx - r,
                            top: cy - r,
                            right: cx + r,
                            bottom: cy + r,
                        },
                        radiusX: r,
                        radiusY: r,
                    },
                    brush,
                );
            }
            Ok(())
        })
    }

    /// One chip filling `client`.
    pub fn chip(&self, hdc: HDC, client: Px, text: &str, on: bool, pal: &Palette) -> Result<()> {
        self.draw(hdc, client, |rt, brush| {
            // SAFETY: drawing calls between BeginDraw and EndDraw.
            unsafe {
                rt.Clear(Some(&color(pal.card)));
                let shape = rounded(client, 4.0 * self.scale);
                brush.SetColor(&color(if on { pal.accent } else { pal.field }));
                rt.FillRoundedRectangle(&shape, brush);
                if !on {
                    brush.SetColor(&color(pal.border));
                    rt.DrawRoundedRectangle(&shape, brush, 1.0, None);
                }
                brush.SetColor(&color(if on { pal.on_accent } else { pal.text }));
            }
            self.text(rt, brush, text, &self.formats.chip, client);
            Ok(())
        })
    }

    /// The rounded frame of a number field; the EDIT control sits inside it.
    pub fn field(&self, hdc: HDC, r: Px, focused: bool, pal: &Palette) -> Result<()> {
        // Drawn on the window's own DC during `page`'s paint pass, so only the frame here.
        self.draw(hdc, r, |rt, brush| {
            // SAFETY: drawing calls between BeginDraw and EndDraw.
            unsafe {
                rt.Clear(Some(&color(pal.card)));
                let local = Px { x: 0, y: 0, ..r };
                let shape = rounded(local, 4.0 * self.scale);
                brush.SetColor(&color(pal.field));
                rt.FillRoundedRectangle(&shape, brush);
                brush.SetColor(&color(pal.border));
                rt.DrawRoundedRectangle(&shape, brush, 1.0, None);
                let line = (if focused { 2.0 } else { 1.0 } * self.scale)
                    .round()
                    .max(1.0);
                brush.SetColor(&color(if focused { pal.accent } else { pal.off_border }));
                rt.FillRectangle(
                    &D2D_RECT_F {
                        left: 3.0 * self.scale,
                        top: local.h as f32 - line,
                        right: local.w as f32 - 3.0 * self.scale,
                        bottom: local.h as f32,
                    },
                    brush,
                );
            }
            Ok(())
        })
    }

    fn metrics(&self, text: &str, role: TextRole, max_width: f32) -> DWRITE_TEXT_METRICS {
        let wide: Vec<u16> = text.encode_utf16().collect();
        let mut m = DWRITE_TEXT_METRICS::default();
        // SAFETY: valid factory and format; metrics is a valid out-pointer.
        unsafe {
            if let Ok(layout) =
                self.dwrite
                    .CreateTextLayout(&wide, self.formats.get(role), max_width, f32::MAX)
            {
                let _ = layout.GetMetrics(&mut m);
            }
        }
        m
    }
}

impl Measure for Painter {
    fn width(&self, text: &str, role: TextRole) -> f32 {
        self.metrics(text, role, f32::MAX)
            .widthIncludingTrailingWhitespace
            / self.scale
    }

    fn height(&self, text: &str, role: TextRole, max_width: f32) -> f32 {
        let max = if max_width.is_finite() {
            max_width * self.scale
        } else {
            f32::MAX
        };
        self.metrics(text, role, max).height / self.scale
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knob_moves_and_grows_when_on() {
        let r = Px {
            x: 0,
            y: 0,
            w: 40,
            h: 20,
        };
        assert_eq!(knob(r, true), (30.0, 10.0, 6.0));
        assert_eq!(knob(r, false), (9.0, 10.0, 5.0));
        let big = Px {
            x: 0,
            y: 0,
            w: 60,
            h: 30,
        };
        assert_eq!(knob(big, true), (45.0, 15.0, 9.0));
    }
}
