//! Byte scans shared by the catalog text rewrite and the AST rewrite gate.

pub(in crate::parser::utils) fn skip_ascii_whitespace(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

pub(in crate::parser::utils) fn is_sql_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

pub(in crate::parser::utils) fn is_sql_ident_part(byte: u8) -> bool {
    is_sql_ident_start(byte) || byte.is_ascii_digit()
}

pub(in crate::parser::utils) fn matching_paren(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            },
            _ => {},
        }
        i += 1;
    }
    None
}

pub(in crate::parser::utils) fn has_ascii_ignore_case(haystack: &str, needle: &str) -> bool {
    find_ignore_ascii_case(haystack, needle).is_some()
}

pub(in crate::parser::utils) fn find_ignore_ascii_case(
    haystack: &str,
    needle: &str,
) -> Option<usize> {
    let bytes = haystack.as_bytes();
    let needle = needle.as_bytes();
    if needle.is_empty() || bytes.len() < needle.len() {
        return None;
    }
    let last = bytes.len() - needle.len();
    let mut i = 0;
    while i <= last {
        if bytes[i..i + needle.len()].eq_ignore_ascii_case(needle) {
            return Some(i);
        }
        i += 1;
    }
    None
}

#[inline]
pub(in crate::parser::utils) fn starts_with_ignore_ascii_case(
    haystack: &[u8],
    offset: usize,
    needle: &[u8],
) -> bool {
    let end = offset + needle.len();
    end <= haystack.len() && haystack[offset..end].eq_ignore_ascii_case(needle)
}
