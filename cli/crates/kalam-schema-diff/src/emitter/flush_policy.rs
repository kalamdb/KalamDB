use crate::{
    model::{Table, FLUSH_POLICY_OPTION},
    sql::same_option_value,
};

pub(super) fn emit_flush_policy_change(current: &Table, target: &Table) -> Option<String> {
    let current_value = current.options.get(FLUSH_POLICY_OPTION);
    let target_value = target.options.get(FLUSH_POLICY_OPTION);

    match (current_value, target_value) {
        (None, None) => None,
        (Some(current_value), Some(target_value))
            if same_option_value(current_value, target_value) =>
        {
            None
        },
        (_, Some(target_value)) => Some(format!(
            "ALTER TABLE {} SET TBLPROPERTIES ({FLUSH_POLICY_OPTION} = {target_value});",
            target.name_sql
        )),
        (Some(_), None) => Some(format!(
            "ALTER TABLE {} SET TBLPROPERTIES ({FLUSH_POLICY_OPTION} = NULL);",
            target.name_sql
        )),
    }
}
