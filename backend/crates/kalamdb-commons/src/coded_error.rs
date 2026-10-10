//! Coded errors: a stable code, a message template, and the arguments.
//!
//! Add an error by appending one line to [`error_catalog`]. That line is the
//! code clients match on, the template endpoints render, the SQLSTATE, and the
//! HTTP status. Call sites store the code and arguments. HTTP, the wire
//! protocol, and function exceptions render the template once.
//!
//! The value you construct is the kind callers branch on, same as a typed
//! exception. [`CodedError::with_context`] pushes a later frame, such as the
//! transaction being committed. The public code stays the original kind.
//!
//! Templates use `%s` for text and `%d` for an integer. `%%` is a literal percent.

use std::fmt::{self, Write};

use smallvec::SmallVec;

const RAFT_PREFIX: &str = "kdb.err/1\n";

macro_rules! error_catalog {
    ($($variant:ident, $code:literal, $template:literal, $kind:ident, $sqlstate:expr, $status:literal);* $(;)?) => {
        /// Stable client code. `Copy`, so matching on it does not allocate.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum ErrorCode {
            $($variant),*
        }

        impl ErrorCode {
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $code,)*
                }
            }

            pub const fn template(self) -> &'static str {
                match self {
                    $(Self::$variant => $template,)*
                }
            }

            /// Location frames are not a code a client branches on.
            pub const fn is_context(self) -> bool {
                match self {
                    $(Self::$variant => error_catalog!(@context $kind),)*
                }
            }

            /// Code a client can branch on. Context and placeholder text have none.
            pub const fn client_code(self) -> Option<&'static str> {
                match self {
                    $(Self::$variant => error_catalog!(@client $kind, $code),)*
                }
            }

            /// PostgreSQL SQLSTATE when this code has a standard equivalent.
            pub const fn sqlstate(self) -> Option<&'static str> {
                match self {
                    $(Self::$variant => $sqlstate,)*
                }
            }

            /// HTTP status written at the API boundary.
            pub const fn http_status(self) -> u16 {
                match self {
                    $(Self::$variant => $status,)*
                }
            }

            pub fn parse(code: &str) -> Option<Self> {
                match code {
                    $($code => Some(Self::$variant),)*
                    _ => None,
                }
            }
        }
    };
    (@context context) => { true };
    (@context cause) => { false };
    (@context opaque) => { false };
    (@client cause, $code:literal) => { Some($code) };
    (@client context, $code:literal) => { None };
    (@client opaque, $code:literal) => { None };
}

error_catalog! {
    NotNullViolation, "NOT_NULL_VIOLATION", "column '%s' cannot be NULL (row %d)", cause, Some("23502"), 400;
    MissingColumn, "MISSING_COLUMN", "column '%s' is missing in row %d", cause, Some("23502"), 400;
    CommitFailed, "COMMIT_FAILED", "while committing transaction '%s'", context, None, 400;
    Detail, "DETAIL", "%s", opaque, None, 400;
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One template argument. The sentence is rendered later, from the template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorArg {
    Text(String),
    Int(i64),
}

impl ErrorArg {
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    pub fn int(value: i64) -> Self {
        Self::Int(value)
    }
}

/// An error that remembers its code and arguments instead of a rendered sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodedError {
    code:     ErrorCode,
    args:     SmallVec<[ErrorArg; 4]>,
    source:   Option<Box<CodedError>>,
    /// The sentence was already rendered (for example after a function exception).
    verbatim: bool,
}

impl CodedError {
    pub fn new(code: ErrorCode, args: impl IntoIterator<Item = ErrorArg>) -> Self {
        Self {
            code,
            args: args.into_iter().collect(),
            source: None,
            verbatim: false,
        }
    }

    pub fn not_null(column: impl Into<String>, row: i64) -> Self {
        Self::new(ErrorCode::NotNullViolation, [ErrorArg::text(column), ErrorArg::int(row)])
    }

    pub fn missing_column(column: impl Into<String>, row: i64) -> Self {
        Self::new(ErrorCode::MissingColumn, [ErrorArg::text(column), ErrorArg::int(row)])
    }

    pub fn detail(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Detail, [ErrorArg::text(message)])
    }

    pub fn pre_rendered(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            args: smallvec::smallvec![ErrorArg::text(message)],
            source: None,
            verbatim: true,
        }
    }

    pub fn with_context(self, code: ErrorCode, args: impl IntoIterator<Item = ErrorArg>) -> Self {
        Self {
            code,
            args: args.into_iter().collect(),
            source: Some(Box::new(self)),
            verbatim: false,
        }
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }

    pub fn leaf(&self) -> &Self {
        let mut current = self;
        while let Some(source) = current.source.as_deref() {
            current = source;
        }
        current
    }

    pub fn leaf_code(&self) -> ErrorCode {
        self.leaf().code
    }

    /// Cause first, then each later frame, such as the transaction being committed.
    pub fn user_message(&self) -> String {
        self.rendered()
    }

    /// One line per frame, for a function exception's `cause`.
    pub fn debug_chain(&self) -> String {
        let mut lines = Vec::new();
        let mut current = Some(self);
        while let Some(frame) = current {
            let mut line = String::with_capacity(frame.code.as_str().len() + 2 + 48);
            line.push_str(frame.code.as_str());
            line.push_str(": ");
            let written = line.len();
            if frame.write_own(&mut line).is_err() {
                line.truncate(written);
                for arg in &frame.args {
                    line.push(' ');
                    push_arg(&mut line, arg);
                }
            }
            lines.push(line);
            current = frame.source.as_deref();
        }
        lines.reverse();
        lines.join("\n")
    }

    /// Compact form stored in a Raft response. Not a user-facing message.
    pub fn encode(&self) -> String {
        let mut frames = Vec::new();
        let mut current = Some(self);
        while let Some(frame) = current {
            frames.push(frame);
            current = frame.source.as_deref();
        }
        let mut out = String::from(RAFT_PREFIX);
        for frame in frames.into_iter().rev() {
            out.push_str(frame.code.as_str());
            out.push('\n');
            let _ = write!(out, "{}\n", frame.args.len());
            out.push(if frame.verbatim { '1' } else { '0' });
            out.push('\n');
            for arg in &frame.args {
                match arg {
                    ErrorArg::Text(text) => write_field(&mut out, 's', text),
                    ErrorArg::Int(value) => write_field(&mut out, 'i', &value.to_string()),
                }
            }
        }
        out
    }

    pub fn decode(message: &str) -> Option<Self> {
        let rest = message.strip_prefix(RAFT_PREFIX)?;
        let mut cursor = Cursor { rest };
        let mut error = None;
        while !cursor.rest.is_empty() {
            let code_name = cursor.line()?;
            let known = ErrorCode::parse(code_name);
            let count: usize = cursor.line()?.parse().ok()?;
            let verbatim = match cursor.line()? {
                "1" => true,
                "0" => false,
                _ => return None,
            };
            let mut args = SmallVec::new();
            for _ in 0..count {
                let kind = cursor.line()?;
                let len: usize = cursor.line()?.parse().ok()?;
                let text = cursor.take(len)?;
                if !cursor.rest.starts_with('\n') {
                    return None;
                }
                cursor.rest = &cursor.rest[1..];
                let arg = match kind {
                    "s" => ErrorArg::text(text),
                    "i" => ErrorArg::Int(text.parse().ok()?),
                    _ => return None,
                };
                args.push(arg);
            }
            let mut frame = if let Some(code) = known {
                Self {
                    code,
                    args,
                    source: None,
                    verbatim,
                }
            } else {
                Self::detail(unknown_code_text(code_name, &args))
            };
            if let Some(cause) = error {
                frame.source = Some(Box::new(cause));
            }
            error = Some(frame);
        }
        error
    }

    fn rendered(&self) -> String {
        let mut message = String::new();
        if self.write_user_message(&mut message).is_err() {
            return self.fallback_message();
        }
        message
    }

    fn write_user_message(&self, formatter: &mut impl Write) -> fmt::Result {
        self.leaf().write_own(formatter)?;
        self.write_contexts(formatter)
    }

    fn write_contexts(&self, formatter: &mut impl Write) -> fmt::Result {
        if let Some(source) = self.source.as_deref() {
            source.write_contexts(formatter)?;
        }
        if self.source.is_some() {
            formatter.write_char(' ')?;
            self.write_own(formatter)?;
        }
        Ok(())
    }

    fn fallback_message(&self) -> String {
        let mut frames = Vec::new();
        let mut current = Some(self);
        while let Some(frame) = current {
            frames.push(frame);
            current = frame.source.as_deref();
        }
        frames.reverse();
        let mut message = String::new();
        for (index, frame) in frames.iter().enumerate() {
            if index > 0 {
                message.push(' ');
            }
            message.push_str(frame.code.as_str());
            for arg in &frame.args {
                message.push(' ');
                push_arg(&mut message, arg);
            }
        }
        message
    }

    fn write_own(&self, formatter: &mut impl Write) -> fmt::Result {
        if self.verbatim {
            return match self.args.first() {
                Some(ErrorArg::Text(text)) => formatter.write_str(text),
                Some(ErrorArg::Int(value)) => write!(formatter, "{value}"),
                None => Ok(()),
            };
        }
        render_template(self.code.template(), &self.args, formatter)
    }
}

impl fmt::Display for CodedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.rendered())
    }
}

impl std::error::Error for CodedError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|source| source.as_ref() as _)
    }
}

struct Cursor<'a> {
    rest: &'a str,
}

impl<'a> Cursor<'a> {
    fn line(&mut self) -> Option<&'a str> {
        let (line, rest) = self.rest.split_once('\n')?;
        self.rest = rest;
        Some(line)
    }

    fn take(&mut self, len: usize) -> Option<&'a str> {
        let (value, rest) = self.rest.split_at_checked(len)?;
        self.rest = rest;
        Some(value)
    }
}

fn write_field(out: &mut String, kind: char, text: &str) {
    out.push(kind);
    out.push('\n');
    let _ = write!(out, "{}\n", text.len());
    out.push_str(text);
    out.push('\n');
}

fn unknown_code_text(code: &str, args: &[ErrorArg]) -> String {
    let mut message = String::from(code);
    for arg in args {
        message.push(' ');
        push_arg(&mut message, arg);
    }
    message
}

fn push_arg(message: &mut String, arg: &ErrorArg) {
    match arg {
        ErrorArg::Text(text) => message.push_str(text),
        ErrorArg::Int(value) => {
            let _ = write!(message, "{value}");
        },
    }
}

fn write_arg_text(formatter: &mut impl Write, arg: &ErrorArg) -> fmt::Result {
    match arg {
        ErrorArg::Text(text) => formatter.write_str(text),
        ErrorArg::Int(_) => Err(fmt::Error),
    }
}

fn render_template(template: &str, args: &[ErrorArg], formatter: &mut impl Write) -> fmt::Result {
    let mut rest = template;
    let mut index = 0;
    while let Some(position) = rest.find('%') {
        formatter.write_str(&rest[..position])?;
        let Some(spec) = rest.as_bytes().get(position + 1).copied() else {
            formatter.write_char('%')?;
            return Ok(());
        };
        match spec {
            b'%' => formatter.write_char('%')?,
            b's' | b'd' => {
                let Some(arg) = args.get(index) else {
                    return Err(fmt::Error);
                };
                index += 1;
                match spec {
                    b's' => write_arg_text(formatter, arg)?,
                    _ => match arg {
                        ErrorArg::Int(value) => write!(formatter, "{value}")?,
                        ErrorArg::Text(_) => return Err(fmt::Error),
                    },
                }
            },
            _ => {
                formatter.write_char('%')?;
                rest = &rest[position + 1..];
                continue;
            },
        }
        rest = &rest[position + 2..];
    }
    if index != args.len() {
        return Err(fmt::Error);
    }
    formatter.write_str(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_null_during_commit_renders_cause_then_context() {
        let error = CodedError::not_null("id", 1).with_context(
            ErrorCode::CommitFailed,
            [ErrorArg::text("01a11265-8179-7503-ac85-4e2bd481462b")],
        );

        assert_eq!(error.leaf_code(), ErrorCode::NotNullViolation);
        assert_eq!(error.leaf_code().client_code(), Some("NOT_NULL_VIOLATION"));
        assert_eq!(error.leaf_code().sqlstate(), Some("23502"));
        assert_eq!(error.leaf_code().http_status(), 400);
        assert_eq!(
            error.user_message(),
            "column 'id' cannot be NULL (row 1) while committing transaction \
             '01a11265-8179-7503-ac85-4e2bd481462b'"
        );
        assert_eq!(
            error.debug_chain(),
            "NOT_NULL_VIOLATION: column 'id' cannot be NULL (row 1)\nCOMMIT_FAILED: while \
             committing transaction '01a11265-8179-7503-ac85-4e2bd481462b'"
        );
    }

    #[test]
    fn raft_encoding_round_trips_code_and_arguments() {
        let error = CodedError::missing_column("name", 2)
            .with_context(ErrorCode::CommitFailed, [ErrorArg::text("tx-1")]);
        let decoded = CodedError::decode(&error.encode()).expect("encoded error");
        assert_eq!(decoded, error);
        assert_eq!(decoded.user_message(), error.user_message());
    }

    #[test]
    fn plain_text_is_not_a_coded_payload() {
        assert!(CodedError::decode("column 'id' cannot be NULL (row 1)").is_none());
    }

    #[test]
    fn template_mismatch_keeps_the_code_and_arguments() {
        let error = CodedError::new(ErrorCode::NotNullViolation, [ErrorArg::text("id")]);
        let message = error.to_string();
        assert!(message.contains("NOT_NULL_VIOLATION"), "{message}");
        assert!(message.contains("id"), "{message}");
    }

    #[test]
    fn unknown_raft_code_stays_readable() {
        let error = CodedError::missing_column("name", 2)
            .with_context(ErrorCode::CommitFailed, [ErrorArg::text("tx-1")]);
        let encoded = error.encode().replace("MISSING_COLUMN", "FUTURE_CODE");
        let decoded = CodedError::decode(&encoded).expect("unknown code still decodes");
        let message = decoded.user_message();
        assert!(message.contains("FUTURE_CODE"), "{message}");
        assert!(message.contains("name"), "{message}");
        assert!(message.contains("while committing transaction 'tx-1'"), "{message}");
        assert!(!message.contains(RAFT_PREFIX), "{message}");
    }
}
