//! Opt-in Windows startup for the current executable, without a helper script.
#[cfg(windows)]
fn registry(args: &[&str]) -> Result<std::process::Output, String> {
    use std::os::windows::process::CommandExt;
    std::process::Command::new("reg.exe")
        .args(args)
        .creation_flags(0x08000000)
        .output()
        .map_err(|e| e.to_string())
}
#[cfg(windows)]
const KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

pub fn enabled() -> Result<Option<bool>, String> {
    #[cfg(windows)]
    {
        let output = registry(&["query", KEY, "/v", "Kuverta"])?;
        return Ok(Some(output.status.success()));
    }
    #[cfg(not(windows))]
    {
        Ok(None)
    }
}

pub fn set(enabled: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        let output = if enabled {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let value = format!("\"{}\"", exe.display());
            registry(&[
                "add", KEY, "/v", "Kuverta", "/t", "REG_SZ", "/d", &value, "/f",
            ])?
        } else {
            if self::enabled()? == Some(false) {
                return Ok(());
            }
            registry(&["delete", KEY, "/v", "Kuverta", "/f"])?
        };
        if output.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).into_owned())
        }
    }
    #[cfg(not(windows))]
    {
        let _ = enabled;
        Err("Windows startup is not available on this system.".into())
    }
}
