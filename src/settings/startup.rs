#[cfg(target_os = "windows")]
mod platform {
    use anyhow::{Context as _, Result, bail};
    use windows::{
        Win32::System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ, RegCloseKey, RegDeleteValueW,
            RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
        },
        core::{PCWSTR, w},
    };

    const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
    const VALUE_NAME: PCWSTR = w!("MultipleRoblox");
    pub(crate) const START_HIDDEN_FLAG: &str = "--hidden";

    fn to_wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn open_run_key(writable: bool) -> Result<HKEY> {
        let mut key = HKEY::default();
        let access = if writable {
            KEY_WRITE | KEY_READ
        } else {
            KEY_READ
        };
        let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, None, access, &mut key) };
        if status.is_err() {
            bail!("the Windows startup list could not be opened");
        }
        Ok(key)
    }

    pub(crate) fn is_enabled() -> bool {
        let Ok(key) = open_run_key(false) else {
            return false;
        };
        let mut size = 0_u32;
        let status =
            unsafe { RegQueryValueExW(key, VALUE_NAME, None, None, None, Some(&mut size)) };
        unsafe {
            let _ = RegCloseKey(key);
        }
        status.is_ok() && size > 0
    }

    pub(crate) fn set_enabled(enabled: bool, start_hidden: bool) -> Result<()> {
        let key = open_run_key(true)?;
        let result = if enabled {
            write_entry(key, start_hidden)
        } else {
            unsafe {
                let _ = RegDeleteValueW(key, VALUE_NAME);
            }
            Ok(())
        };
        unsafe {
            let _ = RegCloseKey(key);
        }
        result
    }

    fn write_entry(key: HKEY, start_hidden: bool) -> Result<()> {
        let executable = std::env::current_exe().context("locating this executable")?;
        let path = executable.to_string_lossy();
        let command = if start_hidden {
            format!("\"{path}\" {START_HIDDEN_FLAG}")
        } else {
            format!("\"{path}\"")
        };

        let wide = to_wide(&command);
        let bytes = unsafe {
            std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), std::mem::size_of_val(&wide[..]))
        };
        let status = unsafe { RegSetValueExW(key, VALUE_NAME, None, REG_SZ, Some(bytes)) };
        if status.is_err() {
            bail!("the Windows startup entry could not be written");
        }
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use anyhow::Result;

    pub(crate) const START_HIDDEN_FLAG: &str = "--hidden";

    pub(crate) fn is_enabled() -> bool {
        false
    }

    pub(crate) fn set_enabled(_enabled: bool, _start_hidden: bool) -> Result<()> {
        Ok(())
    }
}

pub(crate) use platform::{START_HIDDEN_FLAG, is_enabled, set_enabled};

pub(crate) fn launched_hidden() -> bool {
    std::env::args().any(|argument| argument == START_HIDDEN_FLAG)
}
