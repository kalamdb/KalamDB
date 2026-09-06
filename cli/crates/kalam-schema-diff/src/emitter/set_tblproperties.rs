use crate::{
    model::{Table, FLUSH_POLICY_OPTION},
    sql::same_option_value,
};

pub(super) fn emit_set_tblproperties(current: &Table, target: &Table) -> Option<String> {
    let set_options = target
        .options
        .iter()
        .filter(|(key, _)| key.as_str() != FLUSH_POLICY_OPTION)
        .filter_map(|(key, target_value)| match current.options.get(key) {
            Some(current_value) if same_option_value(current_value, target_value) => None,
            _ => Some(format!("{key} = {target_value}")),
        })
        .collect::<Vec<_>>();

    if set_options.is_empty() {
        return None;
    }

    Some(format!(
        "ALTER TABLE {} SET TBLPROPERTIES ({});",
        target.name_sql,
        set_options.join(", ")
    ))
}

pub(super) fn emit_removed_option_comments(current: &Table, target: &Table, out: &mut Vec<String>) {
    for removed_option in current.options.keys() {
        if removed_option == FLUSH_POLICY_OPTION {
            continue;
        }

        if target.options.contains_key(removed_option) {
            continue;
        }

        out.push(format!(
            "-- manual review required: option {} was removed from table {}",
            removed_option, target.name_sql
        ));
        out.push(format!(
            "-- recommended grammar to add: ALTER TABLE {} RESET TBLPROPERTIES ({});",
            target.name_sql, removed_option
        ));
    }
}
