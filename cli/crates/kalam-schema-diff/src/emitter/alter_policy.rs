use crate::model::Policy;

pub(super) fn emit_alter_policy(current: &Policy, target: &Policy) -> Option<String> {
    if current.command != target.command {
        return None;
    }

    if current.using_sql.is_some() && target.using_sql.is_none() {
        return None;
    }

    if current.with_check_sql.is_some() && target.with_check_sql.is_none() {
        return None;
    }

    let mut parts = Vec::new();

    if current.targets_signature() != target.targets_signature() {
        parts.push(format!("TO {}", target.targets_sql()));
    }

    if current.using_sql != target.using_sql {
        let using_sql = target.using_sql.as_deref()?;
        parts.push(format!("USING ({using_sql})"));
    }

    if current.with_check_sql != target.with_check_sql {
        let with_check_sql = target.with_check_sql.as_deref()?;
        parts.push(format!("WITH CHECK ({with_check_sql})"));
    }

    if parts.is_empty() {
        return Some(String::new());
    }

    Some(format!(
        "ALTER POLICY {} ON {} {};",
        target.name_sql,
        target.table_sql,
        parts.join(" ")
    ))
}
