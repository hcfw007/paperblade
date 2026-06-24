mod merge;

/// Merge the given PDF files (in order) into `output`. Returns the output path.
#[tauri::command]
async fn merge_pdfs(inputs: Vec<String>, output: String) -> Result<String, String> {
    merge::merge_pdfs(&inputs, &output)?;
    Ok(output)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![merge_pdfs])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
