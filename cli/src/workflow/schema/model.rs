//! Generated client language targets.

/// Generated client languages. Adding one means: extend this enum and
/// `parse`/`as_str`, add `schema.targets.<key>` in `kalam.toml`, and add an
/// adapter that reads [`super::output::SchemaEmitInput`] (procedure clients
/// from [`super::procedures::ProcedureCatalog`]). Table/row codecs stay in
/// the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LanguageTarget {
    TypeScript,
    Dart,
    Rust,
    Python,
}

impl LanguageTarget {
    /// Every target, in the order error messages and help text list them.
    pub const ALL: &'static [LanguageTarget] =
        &[Self::TypeScript, Self::Dart, Self::Rust, Self::Python];

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "typescript" | "ts" => Some(Self::TypeScript),
            "dart" | "flutter" => Some(Self::Dart),
            "rust" | "rs" => Some(Self::Rust),
            "python" | "py" => Some(Self::Python),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::TypeScript => "typescript",
            Self::Dart => "dart",
            Self::Rust => "rust",
            Self::Python => "python",
        }
    }
}

/// Canonical target names for error messages, derived from [`LanguageTarget::ALL`]
/// so the list cannot drift when a language is added.
pub fn supported_languages_hint() -> String {
    LanguageTarget::ALL.iter().map(|language| language.as_str()).collect::<Vec<_>>().join(", ")
}

pub fn parse_language_list(values: &[String]) -> Vec<LanguageTarget> {
    values.iter().filter_map(|value| LanguageTarget::parse(value)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_language_aliases() {
        assert_eq!(LanguageTarget::parse("ts"), Some(LanguageTarget::TypeScript));
        assert_eq!(LanguageTarget::parse("dart"), Some(LanguageTarget::Dart));
        assert_eq!(LanguageTarget::parse("flutter"), Some(LanguageTarget::Dart));
        assert_eq!(LanguageTarget::parse("rust"), Some(LanguageTarget::Rust));
        assert_eq!(LanguageTarget::parse("python"), Some(LanguageTarget::Python));
        assert_eq!(LanguageTarget::parse("py"), Some(LanguageTarget::Python));
        assert_eq!(LanguageTarget::parse("cobol"), None);
    }

    #[test]
    fn supported_languages_hint_lists_every_target() {
        let hint = supported_languages_hint();
        for language in LanguageTarget::ALL {
            assert!(hint.contains(language.as_str()), "{hint} is missing {}", language.as_str());
        }
    }
}
