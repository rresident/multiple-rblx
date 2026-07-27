#[cfg(target_os = "windows")]
mod platform {
    use anyhow::{Context as _, Result};
    use windows::{
        Win32::{
            Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WIN32_ERROR},
            System::Threading::{CreateEventW, CreateMutexW, ReleaseMutex},
        },
        core::{PCWSTR, w},
    };

    const SINGLETON_MUTEX: PCWSTR = w!("ROBLOX_singletonMutex");
    const SINGLETON_EVENT: PCWSTR = w!("ROBLOX_singletonEvent");

    enum HeldName {
        Mutex(HANDLE),
        Event(HANDLE),
    }

    pub(crate) struct MultiInstanceGuard {
        held: Vec<HeldName>,
    }

    impl MultiInstanceGuard {
        pub(crate) fn is_armed(&self) -> bool {
            !self.held.is_empty()
        }
    }

    impl Drop for MultiInstanceGuard {
        fn drop(&mut self) {
            for name in self.held.drain(..) {
                match name {
                    HeldName::Mutex(handle) => unsafe {
                        let _ = ReleaseMutex(handle);
                        let _ = CloseHandle(handle);
                    },
                    HeldName::Event(handle) => unsafe {
                        let _ = CloseHandle(handle);
                    },
                }
            }
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    pub(crate) enum ArmOutcome {
        Armed,

        ClientAlreadyRunning,
    }

    pub(crate) fn arm() -> Result<(ArmOutcome, MultiInstanceGuard)> {
        let mut guard = MultiInstanceGuard { held: Vec::new() };

        let mutex = unsafe { CreateMutexW(None, true, SINGLETON_MUTEX) }
            .context("creating the Roblox singleton mutex")?;

        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                let _ = CloseHandle(mutex);
            }
            return Ok((ArmOutcome::ClientAlreadyRunning, guard));
        }
        guard.held.push(HeldName::Mutex(mutex));

        let event = unsafe { CreateEventW(None, true, false, SINGLETON_EVENT) }
            .context("creating the Roblox singleton event")?;
        let existed: WIN32_ERROR = unsafe { GetLastError() };
        if existed == ERROR_ALREADY_EXISTS {
            unsafe {
                let _ = CloseHandle(event);
            }
            drop(guard);
            return Ok((
                ArmOutcome::ClientAlreadyRunning,
                MultiInstanceGuard { held: Vec::new() },
            ));
        }
        guard.held.push(HeldName::Event(event));

        Ok((ArmOutcome::Armed, guard))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn arming_twice_in_one_process_reports_the_second_attempt_as_taken() {
            let (first, guard) = arm().expect("first arm should not error");
            if first == ArmOutcome::ClientAlreadyRunning {
                assert!(!guard.is_armed());
                return;
            }
            assert!(guard.is_armed());

            let (second, second_guard) = arm().expect("second arm should not error");
            assert_eq!(second, ArmOutcome::ClientAlreadyRunning);
            assert!(!second_guard.is_armed());

            drop(guard);

            let (third, third_guard) = arm().expect("third arm should not error");
            assert_eq!(third, ArmOutcome::Armed);
            assert!(third_guard.is_armed());
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use anyhow::Result;

    pub(crate) struct MultiInstanceGuard;

    impl MultiInstanceGuard {
        pub(crate) fn is_armed(&self) -> bool {
            false
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    pub(crate) enum ArmOutcome {
        Armed,
        ClientAlreadyRunning,
    }

    pub(crate) fn arm() -> Result<(ArmOutcome, MultiInstanceGuard)> {
        Ok((ArmOutcome::ClientAlreadyRunning, MultiInstanceGuard))
    }
}

pub(crate) use platform::{ArmOutcome, MultiInstanceGuard, arm};
