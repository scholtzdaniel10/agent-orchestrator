use crate::SNAPSHOT_MAX_CHARS;

/// Append `extra` and keep at most `max_chars` Unicode scalar values, dropping the front.
pub(crate) fn push_capped(buf: &mut String, chars: &mut usize, extra: &str, max_chars: usize) {
    buf.push_str(extra);
    *chars += extra.chars().count();
    if *chars <= max_chars {
        return;
    }
    let skip = *chars - max_chars;
    let cut = buf
        .char_indices()
        .nth(skip)
        .map(|(idx, _)| idx)
        .unwrap_or(buf.len());
    buf.drain(..cut);
    *chars = max_chars;
}

pub(crate) fn tail_chars(text: &str, max_chars: usize) -> String {
    let total = text.chars().count();
    if total <= max_chars {
        return text.to_string();
    }
    let skip = total - max_chars;
    text.chars().skip(skip).collect()
}

/// Decode `input` onto `carry`. Incomplete trailing bytes stay in `carry`.
/// Invalid bytes become U+FFFD and do not stall the stream.
pub(crate) fn decode_utf8(carry: &mut Vec<u8>, input: &[u8]) -> String {
    carry.extend_from_slice(input);
    let mut out = String::new();
    loop {
        if carry.is_empty() {
            break;
        }
        match std::str::from_utf8(carry) {
            Ok(text) => {
                out.push_str(text);
                carry.clear();
                break;
            }
            Err(err) => {
                let valid = err.valid_up_to();
                if valid > 0 {
                    if let Ok(text) = std::str::from_utf8(&carry[..valid]) {
                        out.push_str(text);
                    }
                    carry.drain(..valid);
                }
                match err.error_len() {
                    None => break,
                    Some(len) => {
                        out.push('\u{FFFD}');
                        let drop_n = len.max(1).min(carry.len());
                        carry.drain(..drop_n);
                    }
                }
            }
        }
    }
    out
}

pub(crate) fn initial_chars(text: &str) -> (String, usize) {
    let count = text.chars().count();
    if count <= SNAPSHOT_MAX_CHARS {
        return (text.to_string(), count);
    }
    let trimmed = tail_chars(text, SNAPSHOT_MAX_CHARS);
    (trimmed, SNAPSHOT_MAX_CHARS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_multibyte_is_reassembled() {
        let bytes = "—".as_bytes();
        assert!(bytes.len() > 1);
        let mut carry = Vec::new();
        let first = decode_utf8(&mut carry, &bytes[..1]);
        assert!(first.is_empty());
        assert!(!carry.is_empty());
        let second = decode_utf8(&mut carry, &bytes[1..]);
        assert_eq!(second, "—");
        assert!(carry.is_empty());
    }

    #[test]
    fn invalid_byte_becomes_replacement_and_stream_continues() {
        let mut carry = Vec::new();
        let text = decode_utf8(&mut carry, &[0x61, 0xff, 0x62]);
        assert_eq!(text, "a\u{FFFD}b");
        assert!(carry.is_empty());
    }

    #[test]
    fn capped_push_keeps_the_tail() {
        let mut buf = String::new();
        let mut chars = 0;
        push_capped(&mut buf, &mut chars, &"a".repeat(10), 4);
        push_capped(&mut buf, &mut chars, "xyz", 4);
        assert_eq!(buf, "axyz");
        assert_eq!(chars, 4);
    }
}
