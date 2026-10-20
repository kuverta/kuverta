//! Native Windows directory chooser. Only returns a path; it does not save
//! settings, read documents or create invoice directories.
#[cfg(windows)]
pub fn pick(owner: isize, initial: String) -> Result<Option<String>, String> {
    use windows::{
        core::{w, HRESULT, HSTRING},
        Win32::{Foundation::HWND, System::Com::*, UI::Shell::*},
    };
    // A dedicated STA keeps the modal dialog and COM lifetime off the UI/core
    // threads. All COM interfaces are dropped before CoUninitialize.
    std::thread::spawn(move || unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| e.to_string())?;
        let result = (|| -> windows::core::Result<Option<String>> {
            let dialog: IFileOpenDialog =
                CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER)?;
            dialog.SetOptions(
                dialog.GetOptions()?
                    | FOS_PICKFOLDERS
                    | FOS_FORCEFILESYSTEM
                    | FOS_PATHMUSTEXIST
                    | FOS_NOCHANGEDIR,
            )?;
            dialog.SetTitle(w!("Kuverta – Ablageordner auswählen"))?;
            if std::path::Path::new(&initial).is_dir() {
                let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(initial), None)?;
                dialog.SetFolder(&item)?;
            }
            match dialog.Show(Some(HWND(owner as *mut _))) {
                Ok(()) => (),
                Err(e) if e.code() == HRESULT::from_win32(1223) => return Ok(None),
                Err(e) => return Err(e),
            }
            let path = dialog.GetResult()?.GetDisplayName(SIGDN_FILESYSPATH)?;
            let text = path.to_string();
            CoTaskMemFree(Some(path.0.cast()));
            Ok(Some(text?))
        })();
        CoUninitialize();
        result.map_err(|e| e.to_string())
    })
    .join()
    .map_err(|_| "Folder chooser stopped unexpectedly".to_owned())?
}

#[tauri::command]
pub async fn pick_invoice_folder(
    window: tauri::WebviewWindow,
    initial: String,
) -> Result<Option<String>, String> {
    #[cfg(windows)]
    {
        let owner = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
        tauri::async_runtime::spawn_blocking(move || pick(owner, initial))
            .await
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(windows))]
    {
        let _ = (window, initial);
        Err("Please enter the folder path manually on this platform.".into())
    }
}
