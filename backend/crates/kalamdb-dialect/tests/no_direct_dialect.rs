use std::{fs, path::Path};

#[test]
fn production_paths_do_not_construct_upstream_dialects() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let mut violations = Vec::new();
    for tree in ["backend", "cli"] {
        scan_tree(&root.join(tree), &mut violations);
    }
    assert!(
        violations.is_empty(),
        "production code constructed PostgreSqlDialect or GenericDialect:\n{}",
        violations.join("\n")
    );
}

fn scan_tree(dir: &Path, violations: &mut Vec<String>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if matches!(name.as_ref(), "target" | "tests" | "node_modules" | ".git" | "generated") {
                continue;
            }
            scan_tree(&path, violations);
            continue;
        }
        if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
            continue;
        }
        let Ok(source) = fs::read_to_string(&path) else {
            continue;
        };
        for (line_number, line) in production_lines(&source) {
            if line_constructs_upstream_dialect(&line) {
                violations.push(format!("{}:{line_number}: {line}", path.display()));
            }
        }
    }
}

fn line_constructs_upstream_dialect(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.starts_with("//") {
        return false;
    }
    has_unit_struct(line, "PostgreSqlDialect") || has_unit_struct(line, "GenericDialect")
}

fn has_unit_struct(line: &str, name: &str) -> bool {
    let Some(start) = line.find(name) else {
        return false;
    };
    let rest = line[start + name.len()..].trim_start();
    rest.starts_with('{') && rest[1..].trim_start().starts_with('}')
}

fn production_lines(source: &str) -> Vec<(usize, String)> {
    let mut lines = Vec::new();
    let mut cfg_test_pending = false;
    let mut test_depth: Option<i32> = None;
    let mut depth = 0_i32;
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if test_depth.is_none() {
            let starts_test_module = trimmed.contains("#[cfg(test)]") && trimmed.contains("mod ")
                || cfg_test_pending && trimmed.starts_with("mod ");
            if starts_test_module {
                test_depth = Some(depth);
                cfg_test_pending = false;
            } else if trimmed.starts_with("#[cfg(test)]") {
                cfg_test_pending = true;
            } else if !trimmed.is_empty() && !trimmed.starts_with("//") && !trimmed.starts_with('#')
            {
                cfg_test_pending = false;
            }
        }

        if test_depth.is_none() {
            lines.push((index + 1, line.to_string()));
        }

        depth += line.matches('{').count() as i32;
        depth -= line.matches('}').count() as i32;
        if let Some(start) = test_depth {
            if depth <= start {
                test_depth = None;
            }
        }
    }
    lines
}
