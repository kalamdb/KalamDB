pub mod dart;
pub mod typescript;

use std::path::Path;

use crate::{
    error::{CLIError, Result},
    workflow::schema::model::{LanguageTarget, SchemaSnapshot},
};

pub trait SchemaEmitter {
    fn language(&self) -> LanguageTarget;
    fn emit(&self, snapshot: &SchemaSnapshot) -> Result<String>;
}

pub fn emit_for_language(language: LanguageTarget, snapshot: &SchemaSnapshot) -> Result<String> {
    match language {
        LanguageTarget::TypeScript => typescript::TypeScriptEmitter.emit(snapshot),
        LanguageTarget::Dart => dart::DartEmitter.emit(snapshot),
    }
}

pub fn write_generated_file(path: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)
        .map_err(|e| CLIError::FileError(format!("failed to write '{}': {e}", path.display())))?;
    Ok(())
}
