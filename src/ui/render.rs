//! Direct2D/DirectWrite drawing into a per-pixel-alpha layered window (mockup B).

use super::layout::{ItemMetrics, Layout, Mode};
use super::view::{ItemView, value_text};
use crate::model::bar_color;
use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT,
    D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE,
    D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE, D2D1CreateFactory, ID2D1DCRenderTarget,
    ID2D1Factory,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_METRICS, DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HGDIOBJ,
    ReleaseDC, SelectObject,
};
use windows::Win32::UI::WindowsAndMessaging::{ULW_ALPHA, UpdateLayeredWindow};
use windows::core::{Result, w};

// Sizes in DIPs, from mockup B.
const LABEL_SIZE: f32 = 9.5;
const VALUE_SIZE: f32 = 11.0;
const LABEL_LINE: f32 = 12.0;
const VALUE_LINE: f32 = 15.0;
const BAR_W: f32 = 30.0;
const BAR_H: f32 = 5.0;
const BAR_RADIUS: f32 = 2.5;
const BAR_GAP: f32 = 2.0;
pub const GAP: f32 = 10.0;
pub const SEP_W: f32 = 1.0;
const SEP_H: f32 = 30.0;
const ELLIPSIS: &str = "…";
/// Space between an account label and its gauge (#74).
const BADGE_GAP: f32 = 5.0;
/// Account label badge: horizontal padding and height, square corners.
const BADGE_PAD: f32 = 4.0;
const BADGE_H: f32 = 16.0;

struct Palette {
    label: u32,
    value: u32,
    track: u32,
    separator: u32,
}

const DARK: Palette = Palette {
    label: 0xbbbbbb,
    value: 0xffffff,
    track: 0x333333,
    separator: 0x3a3a3a,
};
const LIGHT: Palette = Palette {
    label: 0x5f5f5f,
    value: 0x1a1a1a,
    track: 0xd0d0d0,
    separator: 0xc8c8c8,
};

fn color(rgb: u32, alpha: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: ((rgb >> 16) & 0xff) as f32 / 255.0,
        g: ((rgb >> 8) & 0xff) as f32 / 255.0,
        b: (rgb & 0xff) as f32 / 255.0,
        a: alpha,
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// DIPs → physical pixels, rounded up.
pub fn to_px(dip: f32, dpi: u32) -> i32 {
    (dip * dpi as f32 / 96.0).ceil() as i32
}

fn to_dip(px: i32, dpi: u32) -> f32 {
    px as f32 * 96.0 / dpi as f32
}

pub struct Renderer {
    d2d: ID2D1Factory,
    dwrite: IDWriteFactory,
    label_fmt: IDWriteTextFormat,
    value_fmt: IDWriteTextFormat,
}

fn text_format(
    dw: &IDWriteFactory,
    size: f32,
    weight: DWRITE_FONT_WEIGHT,
) -> Result<IDWriteTextFormat> {
    // SAFETY: plain factory call; DirectWrite falls back if the family is missing.
    let f = unsafe {
        dw.CreateTextFormat(
            w!("Segoe UI Variable Text"),
            None,
            weight,
            DWRITE_FONT_STYLE_NORMAL,
            DWRITE_FONT_STRETCH_NORMAL,
            size,
            w!(""),
        )?
    };
    // SAFETY: format is valid.
    unsafe {
        f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
        f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
    }
    Ok(f)
}

impl Renderer {
    pub fn new() -> Result<Renderer> {
        // SAFETY: factory creation has no preconditions.
        let d2d: ID2D1Factory =
            unsafe { D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)? };
        let dwrite: IDWriteFactory = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };
        let label_fmt = text_format(&dwrite, LABEL_SIZE, DWRITE_FONT_WEIGHT_NORMAL)?;
        let value_fmt = text_format(&dwrite, VALUE_SIZE, DWRITE_FONT_WEIGHT_SEMI_BOLD)?;
        Ok(Renderer {
            d2d,
            dwrite,
            label_fmt,
            value_fmt,
        })
    }

    fn text_width(&self, text: &str, fmt: &IDWriteTextFormat) -> f32 {
        let mut m = DWRITE_TEXT_METRICS::default();
        // SAFETY: valid string and format; metrics is an out value.
        let ok = unsafe {
            self.dwrite
                .CreateTextLayout(&wide(text), fmt, 1000.0, 100.0)
                .and_then(|layout| layout.GetMetrics(&mut m))
        };
        if ok.is_ok() {
            m.widthIncludingTrailingWhitespace
        } else {
            text.len() as f32 * 7.0
        }
    }

    /// Item widths in physical pixels at `dpi`.
    pub fn measure(&self, items: &[ItemView], dpi: u32) -> Vec<ItemMetrics> {
        items
            .iter()
            .map(|item| {
                let value = self
                    .text_width(&value_text(item.value), &self.value_fmt)
                    .max(BAR_W);
                let label = self.text_width(&item.label, &self.label_fmt);
                let badge = self.badge_width(item);
                ItemMetrics {
                    full_w: to_px(badge + value.max(label), dpi),
                    compact_w: to_px(badge + value, dpi),
                }
            })
            .collect()
    }

    /// Width in DIPs an item's account label takes, gap included; 0 without a label.
    fn badge_width(&self, item: &ItemView) -> f32 {
        item.badge.as_deref().map_or(0.0, |b| {
            self.text_width(b, &self.value_fmt).ceil() + 2.0 * BADGE_PAD + BADGE_GAP
        })
    }

    pub fn ellipsis_px(&self, dpi: u32) -> i32 {
        to_px(self.text_width(ELLIPSIS, &self.value_fmt), dpi)
    }

    /// Draws the strip and pushes it to `hwnd` at screen position `pos` with size `size` (px).
    #[allow(clippy::too_many_arguments)]
    pub fn paint(
        &self,
        hwnd: HWND,
        pos: (i32, i32),
        size: (i32, i32),
        dpi: u32,
        items: &[ItemView],
        metrics: &[ItemMetrics],
        layout: &Layout,
        light: bool,
    ) -> Result<()> {
        let (w, h) = (size.0.max(1), size.1.max(1));
        // SAFETY: GDI objects are created, used and released within this call.
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits = std::ptr::null_mut();
            let result = CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
                .and_then(|dib| {
                    let old = SelectObject(mem, HGDIOBJ(dib.0));
                    let drawn = self
                        .draw(mem, (w, h), dpi, items, metrics, layout, light)
                        .and_then(|_| {
                            let blend = BLENDFUNCTION {
                                BlendOp: AC_SRC_OVER as u8,
                                BlendFlags: 0,
                                SourceConstantAlpha: 255,
                                AlphaFormat: AC_SRC_ALPHA as u8,
                            };
                            UpdateLayeredWindow(
                                hwnd,
                                Some(screen),
                                Some(&POINT { x: pos.0, y: pos.1 }),
                                Some(&SIZE { cx: w, cy: h }),
                                Some(mem),
                                Some(&POINT::default()),
                                COLORREF(0),
                                Some(&blend),
                                ULW_ALPHA,
                            )
                        });
                    SelectObject(mem, old);
                    let _ = DeleteObject(HGDIOBJ(dib.0));
                    drawn
                });
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
            result
        }
    }

    #[allow(clippy::too_many_arguments)]
    unsafe fn draw(
        &self,
        dc: windows::Win32::Graphics::Gdi::HDC,
        (w, h): (i32, i32),
        dpi: u32,
        items: &[ItemView],
        metrics: &[ItemMetrics],
        layout: &Layout,
        light: bool,
    ) -> Result<()> {
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            // Software rendering avoids loading the GPU driver (~60 MB) for a tiny strip.
            r#type: D2D1_RENDER_TARGET_TYPE_SOFTWARE,
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: dpi as f32,
            dpiY: dpi as f32,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let palette = if light { &LIGHT } else { &DARK };
        // SAFETY: dc has a 32-bpp DIB selected covering (w, h).
        unsafe {
            let rt: ID2D1DCRenderTarget = self.d2d.CreateDCRenderTarget(&props)?;
            rt.BindDC(
                dc,
                &RECT {
                    left: 0,
                    top: 0,
                    right: w,
                    bottom: h,
                },
            )?;
            rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            rt.BeginDraw();
            // Nearly transparent fill keeps the whole strip hit-testable for hover and clicks.
            rt.Clear(Some(&color(0, 1.0 / 255.0)));
            let brush = rt.CreateSolidColorBrush(&color(0, 1.0), None)?;
            let set = |rgb: u32, a: f32| brush.SetColor(&color(rgb, a));
            let h_dip = to_dip(h, dpi);
            let block_h = match layout.mode {
                Mode::Full => LABEL_LINE + VALUE_LINE + BAR_GAP + BAR_H,
                Mode::Compact => VALUE_LINE + BAR_GAP + BAR_H,
            };
            let top = ((h_dip - block_h) / 2.0).max(0.0);
            let mut x = 0.0f32;
            for (n, &i) in layout.visible.iter().enumerate() {
                if n > 0 {
                    x += GAP;
                }
                let item = &items[i];
                let mut col_w = to_dip(
                    match layout.mode {
                        Mode::Full => metrics[i].full_w,
                        Mode::Compact => metrics[i].compact_w,
                    },
                    dpi,
                );
                // Account label: a square-cornered badge in the service color with white
                // text, centered on the whole block (#74).
                if let Some(badge) = item.badge.as_deref() {
                    let bw = self.badge_width(item);
                    let by = top + (block_h - BADGE_H) / 2.0;
                    let r = D2D_RECT_F {
                        left: x,
                        top: by,
                        right: x + bw - BADGE_GAP,
                        bottom: by + BADGE_H,
                    };
                    set(bar_color(item.kind, Some(0.0)), 1.0);
                    rt.FillRectangle(&r, &brush);
                    set(0xffffff, 1.0);
                    rt.DrawText(
                        &wide(badge),
                        &self.value_fmt,
                        &r,
                        &brush,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                        DWRITE_MEASURING_MODE_NATURAL,
                    );
                    x += bw;
                    col_w -= bw;
                }
                let mut y = top;
                if layout.mode == Mode::Full {
                    set(palette.label, 1.0);
                    let r = D2D_RECT_F {
                        left: x,
                        top: y,
                        right: x + col_w,
                        bottom: y + LABEL_LINE,
                    };
                    rt.DrawText(
                        &wide(&item.label),
                        &self.label_fmt,
                        &r,
                        &brush,
                        D2D1_DRAW_TEXT_OPTIONS_NONE,
                        DWRITE_MEASURING_MODE_NATURAL,
                    );
                    y += LABEL_LINE;
                }
                set(palette.value, 1.0);
                let r = D2D_RECT_F {
                    left: x,
                    top: y,
                    right: x + col_w,
                    bottom: y + VALUE_LINE,
                };
                rt.DrawText(
                    &wide(&value_text(item.value)),
                    &self.value_fmt,
                    &r,
                    &brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
                y += VALUE_LINE + BAR_GAP;
                let bx = x + (col_w - BAR_W) / 2.0;
                let bar = |right: f32| D2D1_ROUNDED_RECT {
                    rect: D2D_RECT_F {
                        left: bx,
                        top: y,
                        right,
                        bottom: y + BAR_H,
                    },
                    radiusX: BAR_RADIUS,
                    radiusY: BAR_RADIUS,
                };
                set(palette.track, 1.0);
                rt.FillRoundedRectangle(&bar(bx + BAR_W), &brush);
                if let Some(v) = item.value.filter(|v| *v > 0.0) {
                    set(
                        bar_color(item.kind, Some(v)),
                        if item.dim { 0.5 } else { 1.0 },
                    );
                    rt.FillRoundedRectangle(&bar(bx + BAR_W * v.clamp(0.0, 100.0) / 100.0), &brush);
                }
                x += col_w;
                if layout.separator_after == Some(i) {
                    x += GAP;
                    set(palette.separator, 1.0);
                    let sy = (h_dip - SEP_H) / 2.0;
                    rt.FillRectangle(
                        &D2D_RECT_F {
                            left: x,
                            top: sy,
                            right: x + SEP_W,
                            bottom: sy + SEP_H,
                        },
                        &brush,
                    );
                    x += SEP_W;
                }
            }
            if layout.overflow && !layout.visible.is_empty() {
                x += GAP;
                set(palette.value, 1.0);
                let r = D2D_RECT_F {
                    left: x,
                    top: 0.0,
                    right: to_dip(w, dpi),
                    bottom: h_dip,
                };
                rt.DrawText(
                    &wide(ELLIPSIS),
                    &self.value_fmt,
                    &r,
                    &brush,
                    D2D1_DRAW_TEXT_OPTIONS_NONE,
                    DWRITE_MEASURING_MODE_NATURAL,
                );
            }
            rt.EndDraw(None, None)
        }
    }
}
