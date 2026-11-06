//! Copies text to the Windows clipboard as `CF_UNICODETEXT`.

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
