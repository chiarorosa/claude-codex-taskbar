//! Settings window colors: light and dark like Windows 11 Settings, or system colors in High
//! Contrast. Values are 0xRRGGBB.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    pub bg: u32,
    pub card: u32,
    pub border: u32,
    pub text: u32,
    pub sub: u32,
    pub success: u32,
    pub warning: u32,
    pub off_border: u32,
    pub off_knob: u32,
    pub field: u32,
    pub accent: u32,
    pub on_accent: u32,
    /// Setup hint card fill and border.
    pub hint: u32,
    pub hint_border: u32,
    /// System colors only: no brand tiles, no tints.
    pub high_contrast: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SysColors {
    pub window: u32,
    pub window_text: u32,
    pub highlight: u32,
    pub highlight_text: u32,
    pub gray_text: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThemeInputs {
    pub light: bool,
    pub high_contrast: bool,
    pub accent: Option<u32>,
    pub sys: SysColors,
}

const LIGHT: Palette = Palette {
    bg: 0xF3F3F3,
    card: 0xFBFBFB,
    border: 0xE5E5E5,
    text: 0x1B1B1B,
    sub: 0x5F5F5F,
    success: 0x0F7B0F,
    warning: 0x9D5D00,
    off_border: 0x858585,
    off_knob: 0x5D5D5D,
    field: 0xFFFFFF,
    accent: 0x005FB8,
    on_accent: 0xFFFFFF,
    hint: 0,
    hint_border: 0,
    high_contrast: false,
};

const DARK: Palette = Palette {
    bg: 0x202020,
    card: 0x2B2B2B,
    border: 0x1D1D1D,
    text: 0xFFFFFF,
    sub: 0xC5C5C5,
    success: 0x6CCB5F,
    warning: 0xFCE100,
    off_border: 0xCFCFCF,
    off_knob: 0xCFCFCF,
    field: 0x1F1F1F,
    accent: 0x4CC2FF,
    on_accent: 0x000000,
    hint: 0,
    hint_border: 0,
    high_contrast: false,
};

/// `over` blended onto `base` with `share` of `over`.
pub fn blend(base: u32, over: u32, share: f32) -> u32 {
    let ch = |shift: u32| {
        let (b, o) = ((base >> shift) & 0xff, (over >> shift) & 0xff);
        ((b as f32) * (1.0 - share) + (o as f32) * share).round() as u32
    };
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

/// Mixes `rgb` 40% toward white, like the lighter accent Windows uses on dark surfaces.
fn lighten(rgb: u32) -> u32 {
    let mix = |c: u32| c * 3 / 5 + 255 * 2 / 5;
    (mix((rgb >> 16) & 0xff) << 16) | (mix((rgb >> 8) & 0xff) << 8) | mix(rgb & 0xff)
}

pub fn palette(t: &ThemeInputs) -> Palette {
    if t.high_contrast {
        let s = t.sys;
        return Palette {
            bg: s.window,
            card: s.window,
            border: s.window_text,
            text: s.window_text,
            sub: s.gray_text,
            success: s.window_text,
            warning: s.window_text,
            off_border: s.window_text,
            off_knob: s.window_text,
            field: s.window,
            accent: s.highlight,
            on_accent: s.highlight_text,
            hint: s.window,
            hint_border: s.window_text,
            high_contrast: true,
        };
    }
    let base = if t.light { LIGHT } else { DARK };
    let accent = match t.accent {
        Some(a) if t.light => a,
        Some(a) => lighten(a),
        None => base.accent,
    };
    Palette {
        accent,
        hint: blend(base.card, base.warning, 0.12),
        hint_border: blend(base.border, base.warning, 0.4),
        ..base
    }
}

/// Reads the Windows app theme, High Contrast, the accent color and the system colors.
pub fn read_theme() -> ThemeInputs {
    use windows::Win32::Graphics::Dwm::DwmGetColorizationColor;
    use windows::Win32::Graphics::Gdi::{
        COLOR_GRAYTEXT, COLOR_HIGHLIGHT, COLOR_HIGHLIGHTTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT,
        GetSysColor,
    };
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RegGetValueW};
    use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };
    use windows::core::HSTRING;

    let mut data = 1u32;
    let mut size = size_of::<u32>() as u32;
    // SAFETY: data/size describe a valid u32 buffer.
    let light = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize"),
            &HSTRING::from("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut data as *mut u32).cast()),
            Some(&mut size),
        )
    }
    .is_err()
        || data != 0;

    let mut hc = HIGHCONTRASTW {
        cbSize: size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    // SAFETY: hc is sized correctly for SPI_GETHIGHCONTRAST.
    let high_contrast = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            hc.cbSize,
            Some((&mut hc as *mut HIGHCONTRASTW).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    }
    .is_ok()
        && hc.dwFlags.contains(HCF_HIGHCONTRASTON);

    let (mut argb, mut opaque) = (0u32, Default::default());
    // SAFETY: both out-pointers are valid locals.
    let accent = unsafe { DwmGetColorizationColor(&mut argb, &mut opaque) }
        .is_ok()
        .then_some(argb & 0xFF_FFFF);

    // COLORREF is 0x00BBGGRR.
    let sys = |index| {
        // SAFETY: GetSysColor has no preconditions.
        let c = unsafe { GetSysColor(index) };
        ((c & 0xff) << 16) | (c & 0xff00) | ((c >> 16) & 0xff)
    };
    ThemeInputs {
        light,
        high_contrast,
        accent,
        sys: SysColors {
            window: sys(COLOR_WINDOW),
            window_text: sys(COLOR_WINDOWTEXT),
            highlight: sys(COLOR_HIGHLIGHT),
            highlight_text: sys(COLOR_HIGHLIGHTTEXT),
            gray_text: sys(COLOR_GRAYTEXT),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sys() -> SysColors {
        SysColors {
            window: 0x000001,
            window_text: 0x000002,
            highlight: 0x000003,
            highlight_text: 0x000004,
            gray_text: 0x000005,
        }
    }

    fn inputs(light: bool, high_contrast: bool, accent: Option<u32>) -> ThemeInputs {
        ThemeInputs {
            light,
            high_contrast,
            accent,
            sys: sys(),
        }
    }

    fn luma(rgb: u32) -> f32 {
        let (r, g, b) = ((rgb >> 16) & 0xff, (rgb >> 8) & 0xff, rgb & 0xff);
        0.2126 * r as f32 + 0.7152 * g as f32 + 0.0722 * b as f32
    }

    #[test]
    fn light_and_dark_match_the_spec_table() {
        let light = palette(&inputs(true, false, None));
        assert_eq!(
            (light.bg, light.card, light.border, light.text),
            (0xF3F3F3, 0xFBFBFB, 0xE5E5E5, 0x1B1B1B)
        );
        assert_eq!(
            (light.success, light.warning, light.accent, light.on_accent),
            (0x0F7B0F, 0x9D5D00, 0x005FB8, 0xFFFFFF)
        );
        let dark = palette(&inputs(false, false, None));
        assert_eq!(
            (dark.bg, dark.card, dark.border, dark.text),
            (0x202020, 0x2B2B2B, 0x1D1D1D, 0xFFFFFF)
        );
        assert_eq!(
            (dark.success, dark.warning, dark.accent, dark.on_accent),
            (0x6CCB5F, 0xFCE100, 0x4CC2FF, 0x000000)
        );
    }

    #[test]
    fn user_accent_is_used_and_lightened_in_dark() {
        assert_eq!(
            palette(&inputs(true, false, Some(0x0078D4))).accent,
            0x0078D4
        );
        let dark = palette(&inputs(false, false, Some(0x0078D4))).accent;
        assert!(
            luma(dark) > luma(0x0078D4),
            "dark mode uses a lighter accent"
        );
    }

    #[test]
    fn hint_card_is_tinted_normally_and_plain_in_high_contrast() {
        let light = palette(&inputs(true, false, None));
        assert!(!light.high_contrast);
        assert_ne!(light.hint, light.card, "warning tint over the card");
        let hc = palette(&inputs(false, true, None));
        let s = sys();
        assert!(hc.high_contrast);
        assert_eq!((hc.hint, hc.hint_border), (s.window, s.window_text));
    }

    #[test]
    fn high_contrast_uses_system_colors_and_ignores_accent() {
        let p = palette(&inputs(false, true, Some(0xFF0000)));
        let s = sys();
        assert_eq!(
            (p.bg, p.card, p.text, p.accent, p.on_accent, p.sub),
            (
                s.window,
                s.window,
                s.window_text,
                s.highlight,
                s.highlight_text,
                s.gray_text
            )
        );
    }
}
