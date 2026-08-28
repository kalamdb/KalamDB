use crate::model::Policy;

pub(super) fn emit_drop_policy(policy: &Policy, allow_drop: bool, out: &mut Vec<String>) {
    if allow_drop {
        out.push(format!("DROP POLICY {} ON {};", policy.name_sql, policy.table_sql));
        return;
    }

    out.push(format!(
        "-- destructive change skipped: policy {} on {} exists in current schema but not in \
         target schema",
        policy.name_sql, policy.table_sql
    ));
    out.push(format!(
        "-- rerun with destructive changes enabled to emit: DROP POLICY {} ON {};",
        policy.name_sql, policy.table_sql
    ));
}
