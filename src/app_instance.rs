#[cfg(target_os = "windows")]
mod platform {
    use anyhow::{Context as _, Result};
    use windows::{
        Win32::{
            Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
            System::Threading::CreateMutexW,
        },
        core::w,
    };

    const INSTANCE_NAME: windows::core::PCWSTR = w!("Local\\MultipleRoblox.AppInstance");

    pub(crate) struct AppInstance(HANDLE);

    impl AppInstance {
        pub(crate) fn acquire() -> Result<Option<Self>> {
            let handle = unsafe { CreateMutexW(None, false, INSTANCE_NAME) }
                .context("creating the application instance marker")?;
            if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                let _ = unsafe { CloseHandle(handle) };
                crate::tray::request_show_from_other_instance();
                return Ok(None);
            }

            Ok(Some(Self(handle)))
        }
    }

    impl Drop for AppInstance {
        fn drop(&mut self) {
            let _ = unsafe { CloseHandle(self.0) };
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use anyhow::Result;

    pub(crate) struct AppInstance;

    impl AppInstance {
        pub(crate) fn acquire() -> Result<Option<Self>> {
            Ok(Some(Self))
        }
    }
}

pub(crate) use platform::AppInstance;
