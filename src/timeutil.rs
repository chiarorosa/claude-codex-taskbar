//! Timestamp parsing and local-time formatting.

use std::time::{SystemTime, UNIX_EPOCH};
use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
use windows::Win32::Globalization::{GetDateFormatEx, GetTimeFormatEx, TIME_NOSECONDS};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows::core::{PCWSTR, w};

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Parses `YYYY-MM-DDTHH:MM:SS[.frac](Z|±HH:MM)` into unix seconds.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') {
        return None;
    }
    let num = |r| num_in(s, r);
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hour, min, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || min > 59 || sec > 60 {
        return None;
    }
    let mut rest = &s[19..];
    if let Some(frac) = rest.strip_prefix('.') {
        let digits = frac.bytes().take_while(u8::is_ascii_digit).count();
        rest = &frac[digits..];
    }
    let offset = match rest {
        "Z" | "z" => 0,
        _ if rest.len() == 6 && (rest.starts_with('+') || rest.starts_with('-')) => {
            let (oh, om) = (num_in(rest, 1..3)?, num_in(rest, 4..6)?);
            let sign = if rest.starts_with('-') { -1 } else { 1 };
            sign * (oh * 3600 + om * 60)
        }
        _ => return None,
    };
    Some(days_from_civil(year, month, day) * 86400 + hour * 3600 + min * 60 + sec - offset)
}

/// Parses an HTTP date in the IMF-fixdate form every server must send
/// (`Sun, 06 Nov 1994 08:49:37 GMT`) into unix seconds.
pub fn parse_http_date(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 29 || &s[3..5] != ", " || &s[25..] != " GMT" {
        return None;
    }
    if b[7] != b' ' || b[11] != b' ' || b[16] != b' ' || b[19] != b':' || b[22] != b':' {
        return None;
    }
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = MONTHS.iter().position(|m| *m == &s[8..11])? as i64 + 1;
    let num = |r| num_in(s, r);
    let (day, year) = (num(5..7)?, num(12..16)?);
    let (hour, min, sec) = (num(17..19)?, num(20..22)?, num(23..25)?);
    if !(1..=31).contains(&day) || hour > 23 || min > 59 || sec > 60 {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86400 + hour * 3600 + min * 60 + sec)
}

fn num_in(s: &str, r: std::ops::Range<usize>) -> Option<i64> {
    let part = s.get(r)?;
    if part.bytes().all(|c| c.is_ascii_digit()) {
        part.parse().ok()
    } else {
        None
    }
}

fn local_systemtime(unix: i64) -> Option<SYSTEMTIME> {
    // FILETIME counts 100 ns intervals since 1601-01-01.
    let ticks = (unix + 11_644_473_600).checked_mul(10_000_000)? as u64;
    let ft = FILETIME {
        dwLowDateTime: ticks as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    // SAFETY: valid pointers to stack values.
    unsafe {
        FileTimeToSystemTime(&ft, &mut utc).ok()?;
        SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).ok()?;
    }
    Some(local)
}

fn format_time(st: &SYSTEMTIME) -> String {
    let mut buf = [0u16; 64];
    // SAFETY: buffer is valid for its length; null locale name means user default.
    let n = unsafe {
        GetTimeFormatEx(
            PCWSTR::null(),
            TIME_NOSECONDS,
            Some(st),
            PCWSTR::null(),
            Some(&mut buf),
        )
    };
    String::from_utf16_lossy(&buf[..(n.max(1) as usize - 1)])
}

/// Local time like `15:00`, in the user's format.
pub fn format_local_hhmm(unix: i64) -> String {
    local_systemtime(unix)
        .map(|st| format_time(&st))
        .unwrap_or_default()
}

/// Local abbreviated weekday plus time, like `Mon 09:00`.
pub fn format_local_day_hhmm(unix: i64) -> String {
    let Some(st) = local_systemtime(unix) else {
        return String::new();
    };
    let mut buf = [0u16; 32];
    // SAFETY: as above.
    let n = unsafe {
        GetDateFormatEx(
            PCWSTR::null(),
            Default::default(),
            Some(&st),
            w!("ddd"),
            Some(&mut buf),
            PCWSTR::null(),
        )
    };
    let day = String::from_utf16_lossy(&buf[..(n.max(1) as usize - 1)]);
    format!("{day} {}", format_time(&st))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_dates_in_imf_fixdate_form() {
        assert_eq!(
            parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(784111777)
        );
        assert_eq!(
            parse_http_date("Wed, 11 Nov 2026 19:30:00 GMT"),
            Some(1794425400)
        );
        for bad in [
            "",
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun Nov  6 08:49:37 1994",
            "Sun, 06 Nov 1994 08:49:37 UTC",
            "Sun, 06 Foo 1994 08:49:37 GMT",
            "Sun, 32 Nov 1994 08:49:37 GMT",
            "Sun, 06 Nov 1994 24:49:37 GMT",
            "Sun, 06 Nov 1994 08:49:37 GMT ",
        ] {
            assert_eq!(parse_http_date(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn rfc3339_utc() {
        assert_eq!(parse_rfc3339("2026-10-30T15:00:00Z"), Some(1793372400));
    }

    #[test]
    fn rfc3339_fraction_and_offset() {
        assert_eq!(
            parse_rfc3339("2026-10-30T15:00:00.123+00:00"),
            Some(1793372400)
        );
        assert_eq!(parse_rfc3339("2026-10-30T15:00:00-03:00"), Some(1793383200));
        assert_eq!(
            parse_rfc3339("2026-10-30T17:29:59.562269+00:00"),
            Some(1793381399)
        );
    }

    #[test]
    fn rfc3339_rejects_garbage() {
        assert_eq!(parse_rfc3339("yesterday"), None);
        assert_eq!(parse_rfc3339("2026-13-01T00:00:00Z"), None);
        assert_eq!(parse_rfc3339(""), None);
    }

    #[test]
    fn rfc3339_leap_year() {
        assert_eq!(parse_rfc3339("2028-02-29T00:00:00Z"), Some(1835395200));
    }
}
