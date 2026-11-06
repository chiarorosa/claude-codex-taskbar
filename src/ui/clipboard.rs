//! Copies text to the Windows clipboard as `CF_UNICODETEXT`.

use windows::Win32::Foundation::{GlobalFree, HANDLE, HWND};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::CF_UNICODETEXT;
use windows::core::{Error, Result};

/// Replaces the clipboard contents with `text`, owned by `hwnd`.
pub fn set_text(hwnd: HWND, text: &str) -> Result<()> {
    let buf = clipboard_utf16(text);
    // SAFETY: the buffer is sized for `buf`, locked only while copying, and freed by us unless
    // SetClipboardData succeeds, after which the system owns it.
    unsafe {
        let mem = GlobalAlloc(GMEM_MOVEABLE, buf.len() * size_of::<u16>())?;
        let dst = GlobalLock(mem).cast::<u16>();
        if dst.is_null() {
            let err = Error::from_thread();
            let _ = GlobalFree(Some(mem));
            return Err(err);
        }
        std::ptr::copy_nonoverlapping(buf.as_ptr(), dst, buf.len());
        let _ = GlobalUnlock(mem);
        let placed = OpenClipboard(Some(hwnd)).and_then(|()| {
            let set = EmptyClipboard()
                .and_then(|()| SetClipboardData(CF_UNICODETEXT.0.into(), Some(HANDLE(mem.0))));
            let _ = CloseClipboard();
            set
        });
        if placed.is_err() {
            let _ = GlobalFree(Some(mem));
        }
        placed.map(|_| ())
    }
}

/// Null-terminated UTF-16 with CRLF line breaks, the layout `CF_UNICODETEXT` expects.
pub fn clipboard_utf16(text: &str) -> Vec<u16> {
    let mut out = Vec::with_capacity(text.len() + 1);
    let mut prev = 0u16;
    for c in text.encode_utf16() {
        if c == u16::from(b'\n') && prev != u16::from(b'\r') {
            out.push(u16::from(b'\r'));
        }
        out.push(c);
        prev = c;
    }
    out.push(0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(buf: &[u16]) -> String {
        assert_eq!(buf.last(), Some(&0), "must be null-terminated");
        assert_eq!(
            buf.iter().filter(|&&c| c == 0).count(),
            1,
            "single terminator"
        );
        String::from_utf16(&buf[..buf.len() - 1]).unwrap()
    }

    #[test]
    fn ascii_round_trips_with_terminator() {
        assert_eq!(decode(&clipboard_utf16("CPU 23%")), "CPU 23%");
    }

    #[test]
    fn accented_text_and_dash_round_trip() {
        let text = "Configurações — 5h —";
        assert_eq!(decode(&clipboard_utf16(text)), text);
    }

    #[test]
    fn line_breaks_become_crlf() {
        assert_eq!(decode(&clipboard_utf16("a\nb\n\nc")), "a\r\nb\r\n\r\nc");
    }

    #[test]
    fn existing_crlf_is_not_doubled() {
        assert_eq!(decode(&clipboard_utf16("a\r\nb")), "a\r\nb");
    }

    #[test]
    fn empty_text_is_just_the_terminator() {
        assert_eq!(clipboard_utf16(""), vec![0]);
    }
}
