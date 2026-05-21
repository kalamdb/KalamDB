use super::CLISession;

impl CLISession {
    pub(in crate::session) fn request_namespace(&self) -> Option<&str> {
        self.current_namespace.as_deref()
    }

    pub(in crate::session) fn effective_namespace(&self) -> &str {
        self.request_namespace().unwrap_or("default")
    }

    pub(in crate::session) fn current_namespace_label(&self) -> String {
        format!("ns:{}", self.effective_namespace())
    }

    pub(in crate::session) fn parse_namespace_switch(sql: &str) -> Option<String> {
        let trimmed = sql.trim().trim_end_matches(';').trim();
        if trimmed.is_empty() {
            return None;
        }

        if let Some(remainder) = Self::strip_ascii_prefix(trimmed, "USE NAMESPACE") {
            return Self::parse_namespace_identifier(remainder);
        }

        if let Some(remainder) = Self::strip_ascii_prefix(trimmed, "SET NAMESPACE") {
            return Self::parse_namespace_identifier(remainder);
        }

        let remainder = Self::strip_ascii_prefix(trimmed, "USE")?;
        let remainder = remainder.trim();
        if remainder.eq_ignore_ascii_case("NAMESPACE") {
            return None;
        }

        Self::parse_namespace_identifier(remainder)
    }

    pub(in crate::session) fn strip_ascii_prefix<'a>(
        value: &'a str,
        prefix: &str,
    ) -> Option<&'a str> {
        let prefix_len = prefix.len();
        if value.len() < prefix_len || !value[..prefix_len].eq_ignore_ascii_case(prefix) {
            return None;
        }

        Some(&value[prefix_len..])
    }

    fn parse_namespace_identifier(input: &str) -> Option<String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return None;
        }

        let first = trimmed.chars().next()?;
        if matches!(first, '\'' | '"' | '`') {
            return Self::parse_quoted_namespace_identifier(trimmed, first);
        }

        let end = trimmed
            .char_indices()
            .find_map(|(index, ch)| (ch.is_whitespace() || ch == ';').then_some(index))
            .unwrap_or(trimmed.len());

        let namespace = &trimmed[..end];
        if namespace.is_empty() || namespace.contains('.') {
            return None;
        }

        if !trimmed[end..].trim().is_empty() {
            return None;
        }

        Some(namespace.to_string())
    }

    fn parse_quoted_namespace_identifier(input: &str, quote: char) -> Option<String> {
        let mut namespace = String::new();
        let mut chars = input.char_indices().peekable();
        let (_, opening_quote) = chars.next()?;
        if opening_quote != quote {
            return None;
        }

        while let Some((index, ch)) = chars.next() {
            if ch == quote {
                if chars.peek().map(|(_, next)| *next == quote).unwrap_or(false) {
                    namespace.push(quote);
                    chars.next();
                    continue;
                }

                if namespace.is_empty() || namespace.contains('.') {
                    return None;
                }

                if !input[index + ch.len_utf8()..].trim().is_empty() {
                    return None;
                }

                return Some(namespace);
            }

            namespace.push(ch);
        }

        None
    }
}
