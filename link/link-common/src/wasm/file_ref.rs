use wasm_bindgen::prelude::*;

use crate::models::FileRef;

fn parse_file_ref(file_ref_json: &str) -> Result<FileRef, JsValue> {
    FileRef::from_json(file_ref_json).ok_or_else(|| JsValue::from_str("invalid FileRef JSON"))
}

#[wasm_bindgen(js_name = fileRefDownloadUrl)]
pub fn file_ref_download_url(
    file_ref_json: &str,
    base_url: &str,
    namespace: &str,
    table: &str,
) -> Result<String, JsValue> {
    Ok(parse_file_ref(file_ref_json)?.download_url(base_url, namespace, table))
}

#[wasm_bindgen(js_name = fileRefRelativeUrl)]
pub fn file_ref_relative_url(
    file_ref_json: &str,
    namespace: &str,
    table: &str,
) -> Result<String, JsValue> {
    Ok(parse_file_ref(file_ref_json)?.relative_url(namespace, table))
}

#[wasm_bindgen(js_name = fileRefStoredName)]
pub fn file_ref_stored_name(file_ref_json: &str) -> Result<String, JsValue> {
    Ok(parse_file_ref(file_ref_json)?.stored_name())
}

#[wasm_bindgen(js_name = fileRefRelativePath)]
pub fn file_ref_relative_path(file_ref_json: &str) -> Result<String, JsValue> {
    Ok(parse_file_ref(file_ref_json)?.relative_path())
}
