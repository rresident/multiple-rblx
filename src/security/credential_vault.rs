use std::sync::Arc;

use super::SharedSessionVault;

#[cfg(target_os = "windows")]
mod platform {
    use std::{
        ffi::c_void,
        ptr::{self, null_mut},
        slice,
        sync::Arc,
    };

    use anyhow::anyhow;
    use anyhow::{Context as _, Result, bail};
    use secrecy::{ExposeSecret as _, SecretString};
    use windows::{
        Win32::{
            Foundation::{
                CloseHandle, ERROR_NOT_FOUND, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT,
            },
            Security::Credentials::{
                CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
                CREDENTIALW, CredDeleteW, CredEnumerateW, CredFree, CredReadW, CredWriteW,
            },
            System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
        },
        core::{Error as WindowsError, HRESULT, PCWSTR, PWSTR, w},
    };

    use crate::security::{
        MAX_SESSION_BYTES, ReplaceSessionResult, SessionBaseline, SessionVault, StoreSessionResult,
        StoredSessionAccount,
    };

    const PRODUCTION_NAMESPACE: &str = "MultipleRoblox:session";
    const VAULT_MUTEX_NAME: PCWSTR = w!("Local\\MultipleRoblox.SessionVault");
    const VAULT_MUTEX_WAIT_MS: u32 = 2_000;

    #[derive(Clone)]
    pub(super) struct WindowsSessionVault {
        namespace: Arc<str>,
    }

    impl WindowsSessionVault {
        pub(super) fn production() -> Self {
            Self {
                namespace: PRODUCTION_NAMESPACE.into(),
            }
        }

        #[cfg(test)]
        fn isolated(namespace: String) -> Self {
            Self {
                namespace: namespace.into(),
            }
        }

        fn target_name(&self, user_id: u64) -> String {
            format!("{}:{user_id}", self.namespace)
        }

        fn filter(&self) -> String {
            format!("{}:*", self.namespace)
        }

        fn parse_target(&self, target: &str) -> Result<u64> {
            let prefix = format!("{}:", self.namespace);
            let id = target
                .strip_prefix(&prefix)
                .context("credential target is outside the application namespace")?;
            id.parse()
                .context("credential target contains an invalid Roblox account identifier")
        }

        fn account(&self, credential: &CREDENTIALW) -> Result<StoredSessionAccount> {
            let target = wide_pointer_to_string(credential.TargetName)
                .context("reading protected-session target")?;
            let user_id = self.parse_target(&target)?;
            let username = wide_pointer_to_string(credential.UserName)
                .context("reading protected-session username")?;
            if username.is_empty() {
                bail!("protected session has an empty Roblox username");
            }
            let added_at_unix = wide_pointer_to_string(credential.Comment)
                .context("reading protected-session creation time")?
                .parse()
                .context("protected session has an invalid creation time")?;

            Ok(StoredSessionAccount {
                user_id,
                username,
                added_at_unix,
            })
        }

        fn read(&self, user_id: u64) -> Result<Option<(StoredSessionAccount, SecretString)>> {
            let target = wide_null(&self.target_name(user_id))?;
            let mut credential = null_mut();

            let result = unsafe {
                CredReadW(
                    PCWSTR::from_raw(target.as_ptr()),
                    CRED_TYPE_GENERIC,
                    None,
                    &mut credential,
                )
            };
            if let Err(error) = result {
                if is_not_found(&error) {
                    return Ok(None);
                }
                return Err(error).context("reading a protected Roblox session");
            }

            let allocation = CredentialRecord(credential);
            let credential = allocation.credential()?;
            let account = self.account(credential)?;
            if credential.CredentialBlob.is_null() || credential.CredentialBlobSize == 0 {
                bail!("protected Roblox session is empty");
            }

            let bytes = unsafe {
                slice::from_raw_parts(
                    credential.CredentialBlob,
                    credential.CredentialBlobSize as usize,
                )
            }
            .to_vec();
            let session = String::from_utf8(bytes)
                .map_err(|error| {
                    use secrecy::zeroize::Zeroize as _;
                    let mut bytes = error.into_bytes();
                    bytes.zeroize();
                    anyhow!("protected Roblox session is not valid UTF-8")
                })
                .map(SecretString::from)?;

            Ok(Some((account, session)))
        }

        fn write(&self, account: &StoredSessionAccount, session: &SecretString) -> Result<()> {
            let target = wide_null(&self.target_name(account.user_id))?;
            let username = wide_null(&account.username)?;
            let comment = wide_null(&account.added_at_unix.to_string())?;
            let secret = session.expose_secret().as_bytes();

            if secret.is_empty() {
                bail!("Roblox returned an empty session");
            }
            debug_assert_eq!(MAX_SESSION_BYTES, CRED_MAX_CREDENTIAL_BLOB_SIZE as usize);
            if secret.len() > MAX_SESSION_BYTES {
                bail!("Roblox session is too large for Windows Credential Manager");
            }

            let credential = CREDENTIALW {
                Type: CRED_TYPE_GENERIC,
                TargetName: PWSTR(target.as_ptr().cast_mut()),
                Comment: PWSTR(comment.as_ptr().cast_mut()),
                CredentialBlobSize: secret.len() as u32,
                CredentialBlob: secret.as_ptr().cast_mut(),
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                UserName: PWSTR(username.as_ptr().cast_mut()),
                ..Default::default()
            };

            unsafe { CredWriteW(&credential, 0) }
                .context("saving the Roblox session in Windows Credential Manager")
        }
    }

    impl SessionVault for WindowsSessionVault {
        fn list(&self) -> Result<Vec<StoredSessionAccount>> {
            let filter = wide_null(&self.filter())?;
            let mut count = 0;
            let mut credentials: *mut *mut CREDENTIALW = null_mut();

            let result = unsafe {
                CredEnumerateW(
                    PCWSTR::from_raw(filter.as_ptr()),
                    None,
                    &mut count,
                    &mut credentials,
                )
            };
            if let Err(error) = result {
                if is_not_found(&error) {
                    return Ok(Vec::new());
                }
                return Err(error).context("listing protected Roblox sessions");
            }

            let allocation = CredentialArray { credentials, count };
            let mut accounts = Vec::with_capacity(count as usize);

            for credential in allocation.credentials()? {
                let credential = unsafe { credential.as_ref() }
                    .context("Windows returned an empty credential record")?;
                if credential.Type != CRED_TYPE_GENERIC {
                    continue;
                }

                accounts.push(self.account(credential)?);
            }

            accounts.sort_by_key(|account| account.added_at_unix);
            Ok(accounts)
        }

        fn store_if_absent(
            &self,
            account: &StoredSessionAccount,
            session: &SecretString,
        ) -> Result<StoreSessionResult> {
            let _lock = VaultMutex::acquire()?;
            if let Some(existing) = self
                .list()?
                .into_iter()
                .find(|existing| existing.user_id == account.user_id)
            {
                return Ok(StoreSessionResult::AlreadyExists(existing));
            }

            self.write(account, session)?;
            Ok(StoreSessionResult::Stored)
        }

        fn load_session(&self, user_id: u64) -> Result<Option<SecretString>> {
            let _lock = VaultMutex::acquire()?;
            Ok(self.read(user_id)?.map(|(_, session)| session))
        }

        fn snapshot(&self, user_id: u64) -> Result<SessionBaseline> {
            let _lock = VaultMutex::acquire()?;
            Ok(match self.read(user_id)? {
                Some((account, session)) => SessionBaseline::present(account, session),
                None => SessionBaseline::missing(user_id),
            })
        }

        fn replace_if_unchanged(
            &self,
            baseline: &SessionBaseline,
            account: &StoredSessionAccount,
            session: &SecretString,
        ) -> Result<ReplaceSessionResult> {
            let _lock = VaultMutex::acquire()?;
            if baseline.user_id() != account.user_id {
                bail!("replacement account does not match the captured session");
            }

            let current = self.read(account.user_id)?;
            if !baseline.matches(
                current
                    .as_ref()
                    .map(|(account, session)| (account, session)),
            ) {
                return Ok(ReplaceSessionResult::Conflict);
            }

            self.write(account, session)?;
            Ok(ReplaceSessionResult::Replaced)
        }

        fn delete(&self, user_id: u64) -> Result<()> {
            let _lock = VaultMutex::acquire()?;
            let target = wide_null(&self.target_name(user_id))?;

            match unsafe { CredDeleteW(PCWSTR::from_raw(target.as_ptr()), CRED_TYPE_GENERIC, None) }
            {
                Ok(()) => Ok(()),
                Err(error) if is_not_found(&error) => Ok(()),
                Err(error) => Err(error)
                    .context("deleting the Roblox session from Windows Credential Manager"),
            }
        }
    }

    struct VaultMutex(HANDLE);

    impl VaultMutex {
        fn acquire() -> Result<Self> {
            let handle = unsafe { CreateMutexW(None, false, VAULT_MUTEX_NAME) }
                .context("opening the account-storage mutex")?;
            let wait = unsafe { WaitForSingleObject(handle, VAULT_MUTEX_WAIT_MS) };
            if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
                Ok(Self(handle))
            } else if wait == WAIT_TIMEOUT {
                let _ = unsafe { CloseHandle(handle) };
                bail!("account storage is busy")
            } else {
                let _ = unsafe { CloseHandle(handle) };
                bail!("waiting for the account-storage mutex failed")
            }
        }
    }

    impl Drop for VaultMutex {
        fn drop(&mut self) {
            let _ = unsafe { ReleaseMutex(self.0) };
            let _ = unsafe { CloseHandle(self.0) };
        }
    }

    struct CredentialRecord(*mut CREDENTIALW);

    impl CredentialRecord {
        fn credential(&self) -> Result<&CREDENTIALW> {
            unsafe { self.0.as_ref() }.context("Windows returned an empty credential record")
        }
    }

    impl Drop for CredentialRecord {
        fn drop(&mut self) {
            unsafe {
                wipe_blob(self.0);
                if !self.0.is_null() {
                    CredFree(self.0.cast::<c_void>());
                }
            }
        }
    }

    struct CredentialArray {
        credentials: *mut *mut CREDENTIALW,
        count: u32,
    }

    impl CredentialArray {
        fn credentials(&self) -> Result<&[*mut CREDENTIALW]> {
            if self.count == 0 {
                return Ok(&[]);
            }
            if self.credentials.is_null() {
                bail!("Windows returned an empty credential list");
            }

            Ok(unsafe { slice::from_raw_parts(self.credentials, self.count as usize) })
        }
    }

    impl Drop for CredentialArray {
        fn drop(&mut self) {
            unsafe {
                if !self.credentials.is_null() {
                    for credential in slice::from_raw_parts(self.credentials, self.count as usize) {
                        wipe_blob(*credential);
                    }
                    CredFree(self.credentials.cast::<c_void>());
                }
            }
        }
    }

    unsafe fn wipe_blob(credential: *mut CREDENTIALW) {
        let Some(credential) = (unsafe { credential.as_mut() }) else {
            return;
        };
        if !credential.CredentialBlob.is_null() && credential.CredentialBlobSize > 0 {
            unsafe {
                ptr::write_bytes(
                    credential.CredentialBlob,
                    0,
                    credential.CredentialBlobSize as usize,
                );
            }
        }
    }

    fn wide_null(value: &str) -> Result<Vec<u16>> {
        if value.contains('\0') {
            bail!("credential metadata contains an embedded NUL");
        }
        Ok(value.encode_utf16().chain([0]).collect())
    }

    fn wide_pointer_to_string(value: PWSTR) -> Result<String> {
        if value.is_null() {
            bail!("credential metadata is missing");
        }

        unsafe { value.to_string() }.context("credential metadata is not valid UTF-16")
    }

    fn is_not_found(error: &WindowsError) -> bool {
        error.code() == HRESULT::from_win32(ERROR_NOT_FOUND.0)
    }

    #[cfg(test)]
    mod tests {
        use std::{
            sync::{Mutex, mpsc},
            thread,
            time::Duration,
        };

        use secrecy::ExposeSecret as _;
        use windows::Win32::Security::Credentials::CRED_MAX_USERNAME_LENGTH;

        use super::*;

        static VAULT_TEST_LOCK: Mutex<()> = Mutex::new(());

        struct Cleanup {
            vault: WindowsSessionVault,
            user_id: u64,
        }

        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = self.vault.delete(self.user_id);
            }
        }

        fn test_vault(label: &str, user_id: u64) -> (WindowsSessionVault, Cleanup) {
            let vault = WindowsSessionVault::isolated(format!(
                "MultipleRoblox:test:{label}:{}",
                std::process::id()
            ));
            vault.delete(user_id).expect("clearing test credential");
            let cleanup = Cleanup {
                vault: vault.clone(),
                user_id,
            };
            (vault, cleanup)
        }

        fn assert_stored(
            vault: &WindowsSessionVault,
            account: &StoredSessionAccount,
            session: &str,
        ) {
            assert_eq!(
                vault.list().expect("listing credentials"),
                vec![account.clone()]
            );
            assert_eq!(
                vault
                    .load_session(account.user_id)
                    .expect("reading credential")
                    .expect("credential should exist")
                    .expose_secret(),
                session
            );
        }

        fn store(
            vault: &WindowsSessionVault,
            account: &StoredSessionAccount,
            session: &SecretString,
        ) {
            assert_eq!(
                vault
                    .store_if_absent(account, session)
                    .expect("storing credential"),
                StoreSessionResult::Stored
            );
        }

        #[test]
        fn round_trips_and_deletes_a_generic_windows_credential() {
            let _test_lock = VAULT_TEST_LOCK
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let user_id = 42;
            let (vault, _cleanup) = test_vault("round-trip", user_id);
            let account = StoredSessionAccount {
                user_id,
                username: "test_account".into(),
                added_at_unix: 1_700_000_000,
            };
            let secret = SecretString::from("unit-test-session".to_owned());

            assert_eq!(
                vault
                    .store_if_absent(&account, &secret)
                    .expect("storing credential"),
                StoreSessionResult::Stored
            );

            assert_eq!(vault.list().expect("listing credentials"), vec![account]);
            assert_eq!(
                vault
                    .load_session(user_id)
                    .expect("reading credential")
                    .expect("credential should exist")
                    .expose_secret(),
                "unit-test-session"
            );

            vault.delete(user_id).expect("deleting credential");
            assert!(
                vault
                    .load_session(user_id)
                    .expect("reading after delete")
                    .is_none()
            );
        }

        #[test]
        fn store_if_absent_preserves_the_original_credential() {
            let _test_lock = VAULT_TEST_LOCK
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let user_id = 84;
            let (vault, _cleanup) = test_vault("no-overwrite", user_id);
            let original = StoredSessionAccount {
                user_id,
                username: "original_account".into(),
                added_at_unix: 1_700_000_000,
            };
            let replacement = StoredSessionAccount {
                user_id,
                username: "replacement_account".into(),
                added_at_unix: 1_800_000_000,
            };

            assert_eq!(
                vault
                    .store_if_absent(
                        &original,
                        &SecretString::from("original-session".to_owned())
                    )
                    .expect("storing original credential"),
                StoreSessionResult::Stored
            );
            assert_eq!(
                vault
                    .store_if_absent(
                        &replacement,
                        &SecretString::from("replacement-session".to_owned())
                    )
                    .expect("rejecting replacement credential"),
                StoreSessionResult::AlreadyExists(original.clone())
            );

            assert_eq!(vault.list().expect("listing credentials"), vec![original]);
            assert_eq!(
                vault
                    .load_session(user_id)
                    .expect("reading preserved credential")
                    .expect("original credential should remain")
                    .expose_secret(),
                "original-session"
            );
        }

        #[test]
        fn replace_if_unchanged_accepts_matching_present_and_missing_baselines() {
            let _test_lock = VAULT_TEST_LOCK
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let user_id = 126;
            let (vault, _cleanup) = test_vault("replace", user_id);
            let first = StoredSessionAccount {
                user_id,
                username: "first_account".into(),
                added_at_unix: 1_700_000_000,
            };
            let second = StoredSessionAccount {
                user_id,
                username: "second_account".into(),
                added_at_unix: 1_800_000_000,
            };

            let missing = vault.snapshot(user_id).expect("capturing missing baseline");
            assert_eq!(
                vault
                    .replace_if_unchanged(
                        &missing,
                        &first,
                        &SecretString::from("first-session".to_owned())
                    )
                    .expect("creating from missing baseline"),
                ReplaceSessionResult::Replaced
            );

            let present = vault.snapshot(user_id).expect("capturing present baseline");
            assert_eq!(
                vault
                    .replace_if_unchanged(
                        &present,
                        &second,
                        &SecretString::from("second-session".to_owned())
                    )
                    .expect("replacing matching credential"),
                ReplaceSessionResult::Replaced
            );
            assert_stored(&vault, &second, "second-session");
        }

        #[test]
        fn replace_if_unchanged_preserves_conflicting_state() {
            let _test_lock = VAULT_TEST_LOCK
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let user_id = 168;
            let (vault, _cleanup) = test_vault("conflicts", user_id);
            let original = StoredSessionAccount {
                user_id,
                username: "original_account".into(),
                added_at_unix: 1_700_000_000,
            };
            let changed = StoredSessionAccount {
                user_id,
                username: "changed_account".into(),
                added_at_unix: 1_800_000_000,
            };
            let candidate = StoredSessionAccount {
                user_id,
                username: "candidate_account".into(),
                added_at_unix: 1_900_000_000,
            };
            let original_session = SecretString::from("original-session".to_owned());
            let changed_session = SecretString::from("changed-session".to_owned());
            let candidate_session = SecretString::from("candidate-session".to_owned());

            store(&vault, &original, &original_session);
            let original_baseline = vault.snapshot(user_id).expect("capturing original");

            vault.delete(user_id).expect("deleting original credential");
            store(&vault, &changed, &original_session);
            assert_eq!(
                vault
                    .replace_if_unchanged(&original_baseline, &candidate, &candidate_session)
                    .expect("checking changed record"),
                ReplaceSessionResult::Conflict
            );
            assert_stored(&vault, &changed, "original-session");

            let changed_baseline = vault.snapshot(user_id).expect("capturing changed record");
            vault.delete(user_id).expect("deleting changed record");
            store(&vault, &changed, &changed_session);
            assert_eq!(
                vault
                    .replace_if_unchanged(&changed_baseline, &candidate, &candidate_session)
                    .expect("checking changed session"),
                ReplaceSessionResult::Conflict
            );
            assert_stored(&vault, &changed, "changed-session");

            let present_baseline = vault.snapshot(user_id).expect("capturing present record");
            vault.delete(user_id).expect("deleting present record");
            assert_eq!(
                vault
                    .replace_if_unchanged(&present_baseline, &candidate, &candidate_session)
                    .expect("checking deleted record"),
                ReplaceSessionResult::Conflict
            );
            assert!(
                vault
                    .load_session(user_id)
                    .expect("reading deleted credential")
                    .is_none()
            );

            let missing_baseline = vault.snapshot(user_id).expect("capturing missing record");
            store(&vault, &changed, &changed_session);
            assert_eq!(
                vault
                    .replace_if_unchanged(&missing_baseline, &candidate, &candidate_session)
                    .expect("checking re-added record"),
                ReplaceSessionResult::Conflict
            );
            assert_stored(&vault, &changed, "changed-session");
        }

        #[test]
        fn failed_cred_write_preserves_the_original_credential() {
            let _test_lock = VAULT_TEST_LOCK
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let user_id = 210;
            let (vault, _cleanup) = test_vault("write-failure", user_id);
            let original = StoredSessionAccount {
                user_id,
                username: "original_account".into(),
                added_at_unix: 1_700_000_000,
            };
            let invalid = StoredSessionAccount {
                user_id,
                username: "x".repeat(CRED_MAX_USERNAME_LENGTH as usize + 1),
                added_at_unix: 1_800_000_000,
            };
            let original_session = SecretString::from("original-session".to_owned());

            store(&vault, &original, &original_session);
            let baseline = vault.snapshot(user_id).expect("capturing original");
            vault
                .replace_if_unchanged(
                    &baseline,
                    &invalid,
                    &SecretString::from("replacement-session".to_owned()),
                )
                .expect_err("CredWriteW should reject an oversized username");

            assert_stored(&vault, &original, "original-session");
        }

        #[test]
        fn vault_mutex_wait_is_bounded_and_fails_closed() {
            let _test_lock = VAULT_TEST_LOCK
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let held = VaultMutex::acquire().expect("holding account-storage mutex");
            let (sender, receiver) = mpsc::channel();
            let contender = thread::spawn(move || {
                let outcome = match VaultMutex::acquire() {
                    Ok(_) => Ok(()),
                    Err(error) => Err(error.to_string()),
                };
                let _ = sender.send(outcome);
            });

            let outcome = receiver.recv_timeout(Duration::from_millis(
                u64::from(VAULT_MUTEX_WAIT_MS) + 1_500,
            ));
            drop(held);
            contender.join().expect("mutex contender should not panic");

            assert_eq!(
                outcome
                    .expect("the mutex wait must return instead of blocking indefinitely")
                    .expect_err("a held storage mutex must fail closed"),
                "account storage is busy"
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use anyhow::{Result, bail};
    use secrecy::SecretString;

    use crate::security::{
        ReplaceSessionResult, SessionBaseline, SessionVault, StoredSessionAccount,
    };

    pub(super) struct UnsupportedSessionVault;

    impl SessionVault for UnsupportedSessionVault {
        fn list(&self) -> Result<Vec<StoredSessionAccount>> {
            bail!("secure Roblox account storage currently requires Windows")
        }

        fn load_session(&self, _: u64) -> Result<Option<SecretString>> {
            bail!("secure Roblox account storage currently requires Windows")
        }

        fn snapshot(&self, _: u64) -> Result<SessionBaseline> {
            bail!("secure Roblox account storage currently requires Windows")
        }

        fn store_if_absent(
            &self,
            _: &StoredSessionAccount,
            _: &SecretString,
        ) -> Result<crate::security::StoreSessionResult> {
            bail!("secure Roblox account storage currently requires Windows")
        }

        fn replace_if_unchanged(
            &self,
            _: &SessionBaseline,
            _: &StoredSessionAccount,
            _: &SecretString,
        ) -> Result<ReplaceSessionResult> {
            bail!("secure Roblox account storage currently requires Windows")
        }

        fn delete(&self, _: u64) -> Result<()> {
            bail!("secure Roblox account storage currently requires Windows")
        }
    }
}

pub(crate) fn system_vault() -> SharedSessionVault {
    #[cfg(target_os = "windows")]
    {
        Arc::new(platform::WindowsSessionVault::production())
    }

    #[cfg(not(target_os = "windows"))]
    {
        Arc::new(platform::UnsupportedSessionVault)
    }
}
