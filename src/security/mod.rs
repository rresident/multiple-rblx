mod credential_vault;

use std::{fmt, sync::Arc};

use anyhow::Result;
use secrecy::{ExposeSecret as _, SecretString};

pub(crate) use credential_vault::system_vault;

pub(crate) const MAX_SESSION_BYTES: usize = 2_560;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StoredSessionAccount {
    pub(crate) user_id: u64,
    pub(crate) username: String,
    pub(crate) added_at_unix: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StoreSessionResult {
    Stored,
    AlreadyExists(StoredSessionAccount),
}

pub(crate) struct SessionBaseline {
    state: BaselineState,
}

enum BaselineState {
    Missing(u64),
    Present {
        account: StoredSessionAccount,
        session: SecretString,
    },
}

impl SessionBaseline {
    pub(crate) fn missing(user_id: u64) -> Self {
        Self {
            state: BaselineState::Missing(user_id),
        }
    }

    pub(crate) fn present(account: StoredSessionAccount, session: SecretString) -> Self {
        Self {
            state: BaselineState::Present { account, session },
        }
    }

    pub(crate) fn user_id(&self) -> u64 {
        match &self.state {
            BaselineState::Missing(user_id) => *user_id,
            BaselineState::Present { account, .. } => account.user_id,
        }
    }

    pub(crate) fn matches(&self, current: Option<(&StoredSessionAccount, &SecretString)>) -> bool {
        match (&self.state, current) {
            (BaselineState::Missing(_), None) => true,
            (
                BaselineState::Present { account, session },
                Some((current_account, current_session)),
            ) => {
                account == current_account
                    && session.expose_secret() == current_session.expose_secret()
            }
            _ => false,
        }
    }
}

impl fmt::Debug for SessionBaseline {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = match &self.state {
            BaselineState::Missing(_) => "missing",
            BaselineState::Present { .. } => "present",
        };
        formatter
            .debug_struct("SessionBaseline")
            .field("user_id", &self.user_id())
            .field("state", &state)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReplaceSessionResult {
    Replaced,
    Conflict,
}

pub(crate) trait SessionVault: Send + Sync {
    fn list(&self) -> Result<Vec<StoredSessionAccount>>;
    fn load_session(&self, user_id: u64) -> Result<Option<SecretString>>;
    fn snapshot(&self, user_id: u64) -> Result<SessionBaseline>;
    fn store_if_absent(
        &self,
        account: &StoredSessionAccount,
        session: &SecretString,
    ) -> Result<StoreSessionResult>;
    fn replace_if_unchanged(
        &self,
        baseline: &SessionBaseline,
        account: &StoredSessionAccount,
        session: &SecretString,
    ) -> Result<ReplaceSessionResult>;
    fn delete(&self, user_id: u64) -> Result<()>;
}

pub(crate) type SharedSessionVault = Arc<dyn SessionVault>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_baseline_debug_is_redacted() {
        let baseline = SessionBaseline::present(
            StoredSessionAccount {
                user_id: 42,
                username: "private-username".into(),
                added_at_unix: 1_700_000_000,
            },
            SecretString::from("private-session".to_owned()),
        );

        let debug = format!("{baseline:?}");
        assert_eq!(debug, "SessionBaseline { user_id: 42, state: \"present\" }");
    }
}
