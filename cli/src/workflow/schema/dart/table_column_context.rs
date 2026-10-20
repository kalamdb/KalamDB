use serde::Serialize;

#[derive(Serialize)]
pub struct TableColumnContext {
    pub field_name: String,
    pub sql_name:   String,
    pub dart_type:  String,
}
