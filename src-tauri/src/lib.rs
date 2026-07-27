mod encrypt;
mod merge;
mod pdf;
mod split;

use split::SplitMode;

/// Merge the given PDF files (in order) into `output`. Returns the output path.
#[tauri::command]
async fn merge_pdfs(inputs: Vec<String>, output: String) -> Result<String, String> {
    merge::merge_pdfs(&inputs, &output)?;
    Ok(output)
}

/// Split `input` into several PDFs under `output_dir`. Returns the paths written.
#[tauri::command]
async fn split_pdf(input: String, mode: SplitMode, output_dir: String) -> Result<Vec<String>, String> {
    split::split_pdf(&input, &mode, &output_dir)
}

/// How many pages the given PDF holds.
#[tauri::command]
async fn page_count(input: String) -> Result<u32, String> {
    pdf::page_count(&input)
}

/// Password-protect `input`, writing the locked copy to `output`.
#[tauri::command]
async fn encrypt_pdf(input: String, output: String, password: String) -> Result<String, String> {
    encrypt::encrypt_pdf(&input, &output, &password)?;
    Ok(output)
}

/// Remove the password from `input`, writing a plain copy to `output`.
#[tauri::command]
async fn decrypt_pdf(input: String, output: String, password: String) -> Result<String, String> {
    encrypt::decrypt_pdf(&input, &output, &password)?;
    Ok(output)
}

/// Whether the given PDF is password-protected.
#[tauri::command]
async fn is_encrypted(input: String) -> Result<bool, String> {
    encrypt::is_encrypted(&input)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            merge_pdfs,
            split_pdf,
            page_count,
            encrypt_pdf,
            decrypt_pdf,
            is_encrypted
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
