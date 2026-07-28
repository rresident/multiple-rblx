#[cfg(target_os = "windows")]
mod platform {
    use std::path::PathBuf;

    use anyhow::{Context as _, Result};
    use windows::{
        Win32::{
            System::Com::{
                CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
                CoTaskMemFree, CoUninitialize, IPersistFile,
            },
            UI::Shell::{
                FOLDERID_Programs, IShellLinkW, KNOWN_FOLDER_FLAG, SHGetKnownFolderPath, ShellLink,
            },
        },
        core::{Interface as _, PCWSTR},
    };

    const SHORTCUT_FILE: &str = "Multiple Roblox.lnk";
    const SHORTCUT_TIP: &str = "Run multiple Roblox accounts at the same time";

    struct Com(bool);

    impl Com {
        fn enter() -> Self {
            let status = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            Self(status.is_ok())
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                unsafe { CoUninitialize() };
            }
        }
    }

    fn to_wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn shortcut_path() -> Result<PathBuf> {
        let raw = unsafe { SHGetKnownFolderPath(&FOLDERID_Programs, KNOWN_FOLDER_FLAG(0), None) }
            .context("the Start menu folder could not be located")?;
        let folder = unsafe { raw.to_string() };
        unsafe { CoTaskMemFree(Some(raw.0.cast())) };
        Ok(
            PathBuf::from(folder.context("the Start menu folder path was unreadable")?)
                .join(SHORTCUT_FILE),
        )
    }

    pub(crate) fn is_enabled() -> bool {
        shortcut_path().is_ok_and(|path| path.exists())
    }

    pub(crate) fn set_enabled(enabled: bool) -> Result<()> {
        let path = shortcut_path()?;

        if !enabled {
            return match std::fs::remove_file(&path) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    Err(error).context("the Start menu shortcut could not be removed")
                }
                _ => Ok(()),
            };
        }

        let executable = std::env::current_exe().context("locating this executable")?;
        let _com = Com::enter();

        let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
            .context("the shortcut could not be created")?;

        let target = to_wide(&executable.to_string_lossy());
        unsafe { link.SetPath(PCWSTR(target.as_ptr())) }
            .context("the shortcut target could not be set")?;
        unsafe { link.SetIconLocation(PCWSTR(target.as_ptr()), 0) }
            .context("the shortcut icon could not be set")?;

        if let Some(parent) = executable.parent() {
            let directory = to_wide(&parent.to_string_lossy());
            unsafe { link.SetWorkingDirectory(PCWSTR(directory.as_ptr())) }
                .context("the shortcut folder could not be set")?;
        }

        let tip = to_wide(SHORTCUT_TIP);
        unsafe { link.SetDescription(PCWSTR(tip.as_ptr())) }
            .context("the shortcut description could not be set")?;

        let file: IPersistFile = link.cast().context("the shortcut could not be prepared")?;
        let destination = to_wide(&path.to_string_lossy());
        unsafe { file.Save(PCWSTR(destination.as_ptr()), true) }
            .context("the Start menu shortcut could not be saved")?;

        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use anyhow::Result;

    pub(crate) fn is_enabled() -> bool {
        false
    }

    pub(crate) fn set_enabled(_enabled: bool) -> Result<()> {
        Ok(())
    }
}

pub(crate) use platform::{is_enabled, set_enabled};

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    #[test]
    #[ignore = "writes to the real Start menu folder"]
    fn shortcut_roundtrips_through_the_start_menu() {
        assert!(
            !is_enabled(),
            "a shortcut already exists, refusing to clobber it"
        );

        set_enabled(true).expect("shortcut should be created");
        assert!(is_enabled(), "shortcut should exist after being enabled");

        set_enabled(false).expect("shortcut should be removed");
        assert!(
            !is_enabled(),
            "shortcut should be gone after being disabled"
        );

        set_enabled(false).expect("removing a missing shortcut should be a no-op");
    }
}
