//! TypeScript text helpers for generated schema artifacts.

use std::path::Path;

pub fn jsdoc(comment: Option<&str>, indent: &str) -> String {
    let Some(text) = comment.map(str::trim).filter(|text| !text.is_empty()) else {
        return String::new();
    };
    let safe = text.replace("*/", "* /");
    let mut out = String::new();
    if !safe.contains('\n') {
        out.push_str(indent);
        out.push_str("/** ");
        out.push_str(&safe);
        out.push_str(" */\n");
        return out;
    }
    out.push_str(indent);
    out.push_str("/**\n");
    for line in safe.lines() {
        out.push_str(indent);
        out.push_str(" * ");
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(indent);
    out.push_str(" */\n");
    out
}

pub fn ts_relative_module(from_dir: &Path, target_file: &Path) -> String {
    let target_stem = target_file.with_extension("");
    let mut from_comps: Vec<_> = from_dir.components().collect();
    let mut to_comps: Vec<_> = target_stem.components().collect();
    while !from_comps.is_empty() && !to_comps.is_empty() && from_comps[0] == to_comps[0] {
        from_comps.remove(0);
        to_comps.remove(0);
    }
    let mut parts: Vec<String> = Vec::new();
    for _ in &from_comps {
        parts.push("..".to_string());
    }
    for component in &to_comps {
        parts.push(component.as_os_str().to_string_lossy().into_owned());
    }
    if parts.is_empty() {
        return "./schema".to_string();
    }
    let joined = parts.join("/");
    if joined.starts_with('.') {
        joined
    } else {
        format!("./{joined}")
    }
}
