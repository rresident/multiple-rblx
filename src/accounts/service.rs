use std::{
    fmt,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::Result as AnyResult;
use secrecy::SecretString;

use crate::security::{
    ReplaceSessionResult, SessionBaseline, SharedSessionVault, StoreSessionResult,
    StoredSessionAccount, system_vault,
};

use super::{
    model::{FetchedProfile, LinkedAccountData, SessionHealth},
    roblox::RobloxClient,
};

pub(super) trait RobloxGateway: Send + Sync {
    fn verify_session(
        &self,
        session: &SecretString,
    ) -> Result<VerifiedRobloxAccount, VerifySessionFailure>;
    fn fetch_profiles(&self, user_ids: &[u64]) -> AnyResult<Vec<FetchedProfile>>;
    fn create_authentication_ticket(
        &self,
        session: &SecretString,
    ) -> Result<SecretString, VerifySessionFailure>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct VerifiedRobloxAccount {
    pub(crate) user_id: u64,
    pub(crate) username: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VerifySessionFailure {
    Rejected,
    Unavailable(VerificationIssue),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum VerificationIssue {
    Timeout,
    Dns,
    Tls,
    HttpStatus(u16),
    Io,
    Protocol,
    InvalidResponse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LaunchFailure {
    SignInRequired,
    Unavailable,
    Storage,
}

impl fmt::Display for LaunchFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::SignInRequired => "this account needs to sign in again",
            Self::Unavailable => "Roblox could not be reached",
            Self::Storage => "the saved sign-in could not be read",
        };
        formatter.write_str(message)
    }
}

pub(crate) struct PreparedLaunch {
    user_id: u64,
    ticket: SecretString,
}

impl PreparedLaunch {
    pub(crate) fn ticket(&self) -> &SecretString {
        &self.ticket
    }
}

impl fmt::Debug for PreparedLaunch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedLaunch")
            .field("user_id", &self.user_id)
            .field("ticket", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LinkAccountFailure {
    Rejected,
    Unavailable,
    Duplicate { username: String },
    Storage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ReauthenticateAccountFailure {
    Rejected,
    Unavailable,
    WrongAccount { username: String },
    Conflict,
    Storage,
}

pub(crate) struct PreparedAccountLink {
    record: StoredSessionAccount,
    session: SecretString,
}

pub(crate) struct PendingAccountReauthentication {
    baseline: SessionBaseline,
    target: StoredSessionAccount,
}

pub(crate) struct PreparedAccountReauthentication {
    baseline: SessionBaseline,
    record: StoredSessionAccount,
    session: SecretString,
}

impl fmt::Debug for PreparedAccountLink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedAccountLink")
            .field("record", &self.record)
            .field("session", &"[REDACTED]")
            .finish()
    }
}

impl fmt::Debug for PreparedAccountReauthentication {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedAccountReauthentication")
            .field("baseline", &self.baseline)
            .field("record", &self.record)
            .field("session", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
impl PreparedAccountLink {
    pub(crate) fn for_test(record: StoredSessionAccount, session: SecretString) -> Self {
        Self { record, session }
    }
}

#[cfg(test)]
impl PreparedAccountReauthentication {
    pub(crate) fn for_test(
        baseline: SessionBaseline,
        record: StoredSessionAccount,
        session: SecretString,
    ) -> Self {
        Self {
            baseline,
            record,
            session,
        }
    }
}

impl fmt::Display for LinkAccountFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected => formatter.write_str("Roblox did not accept this sign-in"),
            Self::Unavailable => formatter.write_str("Roblox could not be reached"),
            Self::Duplicate { .. } => formatter.write_str("this account is already connected"),
            Self::Storage => formatter.write_str("Windows could not protect this account"),
        }
    }
}

impl fmt::Display for ReauthenticateAccountFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected => formatter.write_str("Roblox did not accept this sign-in"),
            Self::Unavailable => formatter.write_str("Roblox could not be reached"),
            Self::WrongAccount { .. } => {
                formatter.write_str("this sign-in belongs to a different account")
            }
            Self::Conflict => formatter.write_str("this account changed during sign-in"),
            Self::Storage => formatter.write_str("Windows could not protect this account"),
        }
    }
}

#[derive(Clone)]
pub(crate) struct AccountServices {
    vault: SharedSessionVault,
    roblox: Arc<dyn RobloxGateway>,
}

impl AccountServices {
    pub(crate) fn system() -> Self {
        Self {
            vault: system_vault(),
            roblox: Arc::new(RobloxClient),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_vault_for_test(vault: SharedSessionVault) -> Self {
        Self {
            vault,
            roblox: Arc::new(RobloxClient),
        }
    }

    pub(crate) fn load_accounts(&self) -> AnyResult<Vec<StoredSessionAccount>> {
        self.vault.list()
    }

    pub(super) fn refresh_profiles(&self, user_ids: &[u64]) -> AnyResult<Vec<FetchedProfile>> {
        self.roblox.fetch_profiles(user_ids)
    }

    pub(super) fn check_session(&self, user_id: u64) -> SessionHealth {
        let session = match self.vault.load_session(user_id) {
            Ok(Some(session)) => session,
            Ok(None) => {
                tracing::warn!(account_id = user_id, "stored Roblox session is missing");
                return SessionHealth::LoginRequired;
            }
            Err(error) => {
                tracing::error!(
                    account_id = user_id,
                    reason = %error,
                    "stored Roblox session could not be read"
                );
                return SessionHealth::CredentialUnavailable;
            }
        };

        match self.roblox.verify_session(&session) {
            Ok(account) if account.user_id == user_id => {
                tracing::debug!(account_id = user_id, "stored Roblox session is valid");
                SessionHealth::Valid
            }
            Ok(account) => {
                tracing::warn!(
                    account_id = user_id,
                    authenticated_account_id = account.user_id,
                    "stored Roblox session belongs to a different account"
                );
                SessionHealth::LoginRequired
            }
            Err(VerifySessionFailure::Rejected) => {
                tracing::info!(
                    account_id = user_id,
                    "stored Roblox session requires sign-in"
                );
                SessionHealth::LoginRequired
            }
            Err(VerifySessionFailure::Unavailable(issue)) => {
                tracing::warn!(
                    account_id = user_id,
                    category = ?issue,
                    "stored Roblox session check was unavailable"
                );
                SessionHealth::CheckUnavailable
            }
        }
    }

    pub(crate) fn prepare_link(
        &self,
        session: SecretString,
    ) -> Result<PreparedAccountLink, LinkAccountFailure> {
        tracing::debug!("verifying captured Roblox session");
        let verified = self
            .roblox
            .verify_session(&session)
            .map_err(|failure| match failure {
                VerifySessionFailure::Rejected => {
                    tracing::warn!(category = "rejected", "Roblox session verification failed");
                    LinkAccountFailure::Rejected
                }
                VerifySessionFailure::Unavailable(issue) => {
                    tracing::warn!(category = ?issue, "Roblox verification was unavailable");
                    LinkAccountFailure::Unavailable
                }
            })?;
        tracing::debug!(account_id = verified.user_id, "Roblox session verified");
        let existing = self.vault.list().map_err(|error| {
            tracing::error!(
                reason = %error,
                "secure storage could not be checked for duplicate accounts"
            );
            LinkAccountFailure::Storage
        })?;
        if let Some(account) = existing
            .iter()
            .find(|account| account.user_id == verified.user_id)
        {
            tracing::info!(
                account_id = verified.user_id,
                "verified Roblox account is already connected"
            );
            return Err(LinkAccountFailure::Duplicate {
                username: account.username.clone(),
            });
        }

        let record = StoredSessionAccount {
            user_id: verified.user_id,
            username: verified.username.clone(),
            added_at_unix: now_unix(),
        };

        Ok(PreparedAccountLink { record, session })
    }

    pub(crate) fn commit_link(
        &self,
        prepared: PreparedAccountLink,
    ) -> Result<LinkedAccountData, LinkAccountFailure> {
        let PreparedAccountLink { record, session } = prepared;
        tracing::debug!(
            account_id = record.user_id,
            "committing verified session to secure storage"
        );
        match self.vault.store_if_absent(&record, &session) {
            Ok(StoreSessionResult::Stored) => {}
            Ok(StoreSessionResult::AlreadyExists(existing)) => {
                tracing::info!(
                    account_id = record.user_id,
                    "verified Roblox account became connected before commit"
                );
                return Err(LinkAccountFailure::Duplicate {
                    username: existing.username,
                });
            }
            Err(error) => {
                tracing::error!(
                    account_id = record.user_id,
                    reason = %error,
                    "secure session commit failed"
                );
                return Err(LinkAccountFailure::Storage);
            }
        }
        drop(session);
        tracing::info!(
            account_id = record.user_id,
            "verified Roblox session stored"
        );

        Ok(LinkedAccountData {
            record,
            avatar_png: None,
        })
    }

    pub(crate) fn begin_reauthentication(
        &self,
        target: StoredSessionAccount,
    ) -> Result<PendingAccountReauthentication, ReauthenticateAccountFailure> {
        let baseline = self.vault.snapshot(target.user_id).map_err(|error| {
            tracing::error!(
                account_id = target.user_id,
                reason = %error,
                "secure session snapshot failed"
            );
            ReauthenticateAccountFailure::Storage
        })?;
        Ok(PendingAccountReauthentication { baseline, target })
    }

    pub(crate) fn prepare_reauthentication(
        &self,
        pending: PendingAccountReauthentication,
        session: SecretString,
    ) -> Result<PreparedAccountReauthentication, ReauthenticateAccountFailure> {
        let PendingAccountReauthentication { baseline, target } = pending;
        let verified = self
            .roblox
            .verify_session(&session)
            .map_err(|failure| match failure {
                VerifySessionFailure::Rejected => ReauthenticateAccountFailure::Rejected,
                VerifySessionFailure::Unavailable(_) => ReauthenticateAccountFailure::Unavailable,
            })?;

        if verified.user_id != target.user_id {
            tracing::warn!(
                account_id = target.user_id,
                authenticated_account_id = verified.user_id,
                "Roblox sign-in belongs to a different account"
            );
            return Err(ReauthenticateAccountFailure::WrongAccount {
                username: target.username,
            });
        }

        let record = StoredSessionAccount {
            username: verified.username,
            ..target
        };
        Ok(PreparedAccountReauthentication {
            baseline,
            record,
            session,
        })
    }

    pub(crate) fn commit_reauthentication(
        &self,
        prepared: PreparedAccountReauthentication,
    ) -> Result<LinkedAccountData, ReauthenticateAccountFailure> {
        let PreparedAccountReauthentication {
            baseline,
            record,
            session,
        } = prepared;
        match self
            .vault
            .replace_if_unchanged(&baseline, &record, &session)
        {
            Ok(ReplaceSessionResult::Replaced) => {}
            Ok(ReplaceSessionResult::Conflict) => {
                return Err(ReauthenticateAccountFailure::Conflict);
            }
            Err(error) => {
                tracing::error!(
                    account_id = record.user_id,
                    reason = %error,
                    "secure session replacement failed"
                );
                return Err(ReauthenticateAccountFailure::Storage);
            }
        }
        drop(session);
        tracing::info!(account_id = record.user_id, "Roblox session replaced");

        Ok(LinkedAccountData {
            record,
            avatar_png: None,
        })
    }

    #[cfg(test)]
    fn link(&self, session: SecretString) -> Result<LinkedAccountData, LinkAccountFailure> {
        let prepared = self.prepare_link(session)?;
        self.commit_link(prepared)
    }

    pub(crate) fn prepare_launch(&self, user_id: u64) -> Result<PreparedLaunch, LaunchFailure> {
        let session = match self.vault.load_session(user_id) {
            Ok(Some(session)) => session,
            Ok(None) => {
                tracing::warn!(account_id = user_id, "no stored session to launch with");
                return Err(LaunchFailure::SignInRequired);
            }
            Err(error) => {
                tracing::error!(
                    account_id = user_id,
                    reason = %error,
                    "stored session could not be read for launch"
                );
                return Err(LaunchFailure::Storage);
            }
        };

        match self.roblox.create_authentication_ticket(&session) {
            Ok(ticket) => {
                tracing::debug!(account_id = user_id, "launch ticket acquired");
                Ok(PreparedLaunch { user_id, ticket })
            }
            Err(VerifySessionFailure::Rejected) => {
                tracing::warn!(account_id = user_id, "Roblox rejected the stored session");
                Err(LaunchFailure::SignInRequired)
            }
            Err(VerifySessionFailure::Unavailable(issue)) => {
                tracing::warn!(
                    account_id = user_id,
                    ?issue,
                    "launch ticket could not be acquired"
                );
                Err(LaunchFailure::Unavailable)
            }
        }
    }

    pub(crate) fn vault(&self) -> &SharedSessionVault {
        &self.vault
    }

    pub(crate) fn remove(&self, user_id: u64) -> AnyResult<()> {
        self.vault.delete(user_id)
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use secrecy::ExposeSecret as _;

    use super::*;
    use crate::security::SessionVault;

    #[derive(Default)]
    struct MemoryVault {
        accounts: Mutex<Vec<(StoredSessionAccount, SecretString)>>,
    }

    impl SessionVault for MemoryVault {
        fn list(&self) -> AnyResult<Vec<StoredSessionAccount>> {
            Ok(self
                .accounts
                .lock()
                .expect("memory vault lock")
                .iter()
                .map(|(account, _)| account.clone())
                .collect())
        }

        fn load_session(&self, user_id: u64) -> AnyResult<Option<SecretString>> {
            Ok(self
                .accounts
                .lock()
                .expect("memory vault lock")
                .iter()
                .find(|(account, _)| account.user_id == user_id)
                .map(|(_, session)| SecretString::from(session.expose_secret().to_owned())))
        }

        fn snapshot(&self, user_id: u64) -> AnyResult<SessionBaseline> {
            let accounts = self.accounts.lock().expect("memory vault lock");
            Ok(accounts
                .iter()
                .find(|(account, _)| account.user_id == user_id)
                .map(|(account, session)| {
                    SessionBaseline::present(
                        account.clone(),
                        SecretString::from(session.expose_secret().to_owned()),
                    )
                })
                .unwrap_or_else(|| SessionBaseline::missing(user_id)))
        }

        fn store_if_absent(
            &self,
            account: &StoredSessionAccount,
            session: &SecretString,
        ) -> AnyResult<StoreSessionResult> {
            let mut accounts = self.accounts.lock().expect("memory vault lock");
            if let Some((existing, _)) = accounts
                .iter()
                .find(|(existing, _)| existing.user_id == account.user_id)
            {
                return Ok(StoreSessionResult::AlreadyExists(existing.clone()));
            }
            accounts.push((
                account.clone(),
                SecretString::from(session.expose_secret().to_owned()),
            ));
            Ok(StoreSessionResult::Stored)
        }

        fn replace_if_unchanged(
            &self,
            baseline: &SessionBaseline,
            account: &StoredSessionAccount,
            session: &SecretString,
        ) -> AnyResult<ReplaceSessionResult> {
            let mut accounts = self.accounts.lock().expect("memory vault lock");
            if baseline.user_id() != account.user_id {
                anyhow::bail!("replacement account does not match baseline");
            }

            let index = accounts
                .iter()
                .position(|(current, _)| current.user_id == account.user_id);
            let current = index.map(|index| {
                let (account, session) = &accounts[index];
                (account, session)
            });
            if !baseline.matches(current) {
                return Ok(ReplaceSessionResult::Conflict);
            }

            let replacement = (
                account.clone(),
                SecretString::from(session.expose_secret().to_owned()),
            );
            if let Some(index) = index {
                accounts[index] = replacement;
            } else {
                accounts.push(replacement);
            }
            Ok(ReplaceSessionResult::Replaced)
        }

        fn delete(&self, user_id: u64) -> AnyResult<()> {
            self.accounts
                .lock()
                .expect("memory vault lock")
                .retain(|(account, _)| account.user_id != user_id);
            Ok(())
        }
    }

    impl MemoryVault {
        fn session(&self, user_id: u64) -> Option<SecretString> {
            self.accounts
                .lock()
                .expect("memory vault lock")
                .iter()
                .find(|(account, _)| account.user_id == user_id)
                .map(|(_, session)| SecretString::from(session.expose_secret().to_owned()))
        }
    }

    struct StubRoblox {
        verification: Result<VerifiedRobloxAccount, VerifySessionFailure>,
    }

    impl RobloxGateway for StubRoblox {
        fn verify_session(
            &self,
            _: &SecretString,
        ) -> Result<VerifiedRobloxAccount, VerifySessionFailure> {
            self.verification.clone()
        }

        fn fetch_profiles(&self, _: &[u64]) -> AnyResult<Vec<FetchedProfile>> {
            Ok(Vec::new())
        }

        fn create_authentication_ticket(
            &self,
            _: &SecretString,
        ) -> Result<SecretString, VerifySessionFailure> {
            Ok(SecretString::from("stub-ticket".to_owned()))
        }
    }

    fn services(
        vault: Arc<dyn SessionVault>,
        verification: Result<VerifiedRobloxAccount, VerifySessionFailure>,
    ) -> AccountServices {
        AccountServices {
            vault,
            roblox: Arc::new(StubRoblox { verification }),
        }
    }

    #[test]
    fn verified_session_is_written_once_without_exposing_it_in_metadata() {
        let vault = Arc::new(MemoryVault::default());
        let services = services(
            vault.clone(),
            Ok(VerifiedRobloxAccount {
                user_id: 42,
                username: "account_name".into(),
            }),
        );
        let canary = "canary-secret-that-must-never-be-metadata";

        let linked = services
            .link(SecretString::from(canary.to_owned()))
            .expect("linking account");

        assert_eq!(linked.record.user_id, 42);
        assert_eq!(linked.record.username, "account_name");
        assert!(!format!("{:?}", linked.record).contains(canary));
        assert_eq!(
            vault.session(42).expect("stored session").expose_secret(),
            canary
        );
    }

    #[test]
    fn rejected_session_never_reaches_the_vault() {
        let vault = Arc::new(MemoryVault::default());
        let services = services(vault.clone(), Err(VerifySessionFailure::Rejected));

        assert!(matches!(
            services.link(SecretString::from("rejected".to_owned())),
            Err(LinkAccountFailure::Rejected)
        ));
        assert!(vault.list().expect("listing accounts").is_empty());
    }

    #[test]
    fn stored_session_health_distinguishes_login_from_network_failures() {
        let vault = Arc::new(MemoryVault::default());
        let account = StoredSessionAccount {
            user_id: 42,
            username: "account".into(),
            added_at_unix: 1,
        };
        vault
            .store_if_absent(&account, &SecretString::from("session".to_owned()))
            .expect("seeding session");

        let rejected = services(vault.clone(), Err(VerifySessionFailure::Rejected));
        assert_eq!(rejected.check_session(42), SessionHealth::LoginRequired);

        let unavailable = services(
            vault,
            Err(VerifySessionFailure::Unavailable(
                VerificationIssue::Timeout,
            )),
        );
        assert_eq!(
            unavailable.check_session(42),
            SessionHealth::CheckUnavailable
        );
    }

    #[test]
    fn stored_session_health_rejects_an_account_id_mismatch() {
        let vault = Arc::new(MemoryVault::default());
        let account = StoredSessionAccount {
            user_id: 42,
            username: "account".into(),
            added_at_unix: 1,
        };
        vault
            .store_if_absent(&account, &SecretString::from("session".to_owned()))
            .expect("seeding session");
        let services = services(
            vault,
            Ok(VerifiedRobloxAccount {
                user_id: 7,
                username: "different".into(),
            }),
        );

        assert_eq!(services.check_session(42), SessionHealth::LoginRequired);
    }

    #[test]
    fn duplicate_session_does_not_overwrite_the_existing_credential() {
        let vault = Arc::new(MemoryVault::default());
        let existing = StoredSessionAccount {
            user_id: 42,
            username: "existing".into(),
            added_at_unix: 1,
        };
        vault
            .store_if_absent(&existing, &SecretString::from("original".to_owned()))
            .expect("seeding vault");
        let services = services(
            vault.clone(),
            Ok(VerifiedRobloxAccount {
                user_id: 42,
                username: "new-name".into(),
            }),
        );

        assert!(matches!(
            services.link(SecretString::from("replacement".to_owned())),
            Err(LinkAccountFailure::Duplicate { username }) if username == "existing"
        ));
        assert_eq!(
            vault.session(42).expect("stored session").expose_secret(),
            "original"
        );
    }

    #[test]
    fn reauthentication_rejects_a_different_account_without_replacement() {
        let vault = Arc::new(MemoryVault::default());
        let target = StoredSessionAccount {
            user_id: 42,
            username: "target".into(),
            added_at_unix: 10,
        };
        vault
            .store_if_absent(&target, &SecretString::from("original".to_owned()))
            .expect("seeding vault");
        let services = services(
            vault.clone(),
            Ok(VerifiedRobloxAccount {
                user_id: 7,
                username: "other".into(),
            }),
        );

        let pending = services
            .begin_reauthentication(target.clone())
            .expect("starting reauthentication");
        assert!(matches!(
            services.prepare_reauthentication(
                pending,
                SecretString::from("replacement".to_owned()),
            ),
            Err(ReauthenticateAccountFailure::WrongAccount { username })
                if username == "target"
        ));
        assert_eq!(vault.list().expect("listing accounts"), vec![target]);
        assert_eq!(
            vault.session(42).expect("stored session").expose_secret(),
            "original"
        );
    }

    #[test]
    fn reauthentication_replaces_an_existing_session_and_preserves_added_at() {
        let vault = Arc::new(MemoryVault::default());
        let target = StoredSessionAccount {
            user_id: 42,
            username: "old-name".into(),
            added_at_unix: 10,
        };
        vault
            .store_if_absent(&target, &SecretString::from("original".to_owned()))
            .expect("seeding vault");
        let services = services(
            vault.clone(),
            Ok(VerifiedRobloxAccount {
                user_id: 42,
                username: "current-name".into(),
            }),
        );

        let pending = services
            .begin_reauthentication(target)
            .expect("starting reauthentication");
        let prepared = services
            .prepare_reauthentication(pending, SecretString::from("replacement".to_owned()))
            .expect("preparing reauthentication");
        let linked = services
            .commit_reauthentication(prepared)
            .expect("committing reauthentication");

        assert_eq!(linked.record.user_id, 42);
        assert_eq!(linked.record.username, "current-name");
        assert_eq!(linked.record.added_at_unix, 10);
        assert_eq!(vault.list().expect("listing accounts"), vec![linked.record]);
        assert_eq!(
            vault.session(42).expect("stored session").expose_secret(),
            "replacement"
        );
    }

    #[test]
    fn reauthentication_restores_a_missing_session() {
        let vault = Arc::new(MemoryVault::default());
        let services = services(
            vault.clone(),
            Ok(VerifiedRobloxAccount {
                user_id: 42,
                username: "current-name".into(),
            }),
        );
        let target = StoredSessionAccount {
            user_id: 42,
            username: "old-name".into(),
            added_at_unix: 10,
        };

        let pending = services
            .begin_reauthentication(target)
            .expect("starting reauthentication");
        let prepared = services
            .prepare_reauthentication(pending, SecretString::from("replacement".to_owned()))
            .expect("preparing reauthentication");
        let linked = services
            .commit_reauthentication(prepared)
            .expect("committing reauthentication");

        assert_eq!(linked.record.username, "current-name");
        assert_eq!(linked.record.added_at_unix, 10);
        assert_eq!(vault.list().expect("listing accounts"), vec![linked.record]);
        assert_eq!(
            vault.session(42).expect("stored session").expose_secret(),
            "replacement"
        );
    }

    #[test]
    fn reauthentication_does_not_recreate_a_session_removed_after_start() {
        let vault = Arc::new(MemoryVault::default());
        let target = StoredSessionAccount {
            user_id: 42,
            username: "old-name".into(),
            added_at_unix: 10,
        };
        vault
            .store_if_absent(&target, &SecretString::from("original".to_owned()))
            .expect("seeding vault");
        let services = services(
            vault.clone(),
            Ok(VerifiedRobloxAccount {
                user_id: 42,
                username: "current-name".into(),
            }),
        );

        let pending = services
            .begin_reauthentication(target)
            .expect("starting reauthentication");
        vault.delete(42).expect("removing account");
        let prepared = services
            .prepare_reauthentication(pending, SecretString::from("replacement".to_owned()))
            .expect("preparing reauthentication");

        assert!(matches!(
            services.commit_reauthentication(prepared),
            Err(ReauthenticateAccountFailure::Conflict)
        ));
        assert!(vault.list().expect("listing accounts").is_empty());
        assert!(vault.session(42).is_none());
    }

    #[test]
    fn reauthentication_detects_a_concurrent_session_change() {
        let vault = Arc::new(MemoryVault::default());
        let target = StoredSessionAccount {
            user_id: 42,
            username: "old-name".into(),
            added_at_unix: 10,
        };
        vault
            .store_if_absent(&target, &SecretString::from("original".to_owned()))
            .expect("seeding vault");
        let services = services(
            vault.clone(),
            Ok(VerifiedRobloxAccount {
                user_id: 42,
                username: "current-name".into(),
            }),
        );
        let pending = services
            .begin_reauthentication(target.clone())
            .expect("starting reauthentication");
        let prepared = services
            .prepare_reauthentication(pending, SecretString::from("replacement".to_owned()))
            .expect("preparing reauthentication");
        vault.delete(42).expect("removing original");
        vault
            .store_if_absent(&target, &SecretString::from("concurrent".to_owned()))
            .expect("writing concurrent session");

        assert!(matches!(
            services.commit_reauthentication(prepared),
            Err(ReauthenticateAccountFailure::Conflict)
        ));
        assert_eq!(
            vault.session(42).expect("stored session").expose_secret(),
            "concurrent"
        );
    }

    #[test]
    fn prepared_reauthentication_debug_is_redacted() {
        let vault = Arc::new(MemoryVault::default());
        let target = StoredSessionAccount {
            user_id: 42,
            username: "account".into(),
            added_at_unix: 10,
        };
        let old_secret = "old-secret-canary";
        let new_secret = "new-secret-canary";
        vault
            .store_if_absent(&target, &SecretString::from(old_secret.to_owned()))
            .expect("seeding vault");
        let services = services(
            vault,
            Ok(VerifiedRobloxAccount {
                user_id: 42,
                username: "account".into(),
            }),
        );

        let pending = services
            .begin_reauthentication(target)
            .expect("starting reauthentication");
        let prepared = services
            .prepare_reauthentication(pending, SecretString::from(new_secret.to_owned()))
            .expect("preparing reauthentication");
        let debug = format!("{prepared:?}");

        assert!(!debug.contains(old_secret));
        assert!(!debug.contains(new_secret));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn unavailable_verifier_returns_a_redacted_error() {
        let services = services(
            Arc::new(MemoryVault::default()),
            Err(VerifySessionFailure::Unavailable(
                VerificationIssue::Timeout,
            )),
        );
        let secret = "never-print-this-cookie";

        let error = services
            .link(SecretString::from(secret.to_owned()))
            .expect_err("verification should fail");

        assert_eq!(error, LinkAccountFailure::Unavailable);
        assert!(!format!("{error:?}").contains(secret));
        assert!(!error.to_string().contains(secret));
    }
}
