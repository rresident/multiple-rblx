use std::{
    collections::HashMap,
    fmt,
    sync::{Arc, Mutex, MutexGuard},
};

use gpui::{Image, ImageFormat, SharedString};
use time::{OffsetDateTime, UtcOffset};

use crate::security::StoredSessionAccount;

use super::instance::{InstancePhase, InstanceRegistry, TransitionToken};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SessionHealth {
    Checking,
    Valid,
    LoginRequired,
    CheckUnavailable,
    CredentialUnavailable,
}

#[derive(Clone)]
pub(crate) struct AccountRowToken {
    pub(super) account_id: u64,
    generation: u64,
    authorization: Arc<Mutex<bool>>,
}

pub(super) type SessionCheckToken = AccountRowToken;

impl AccountRowToken {
    pub(crate) fn is_authorized(&self) -> bool {
        *self
            .authorization
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn hold_authorization(&self) -> Option<MutexGuard<'_, bool>> {
        let authorization = self
            .authorization
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if *authorization {
            Some(authorization)
        } else {
            None
        }
    }

    fn revoke(&self) {
        *self
            .authorization
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = false;
    }

    #[cfg(test)]
    pub(crate) fn for_test(account_id: u64) -> Self {
        Self {
            account_id,
            generation: 1,
            authorization: Arc::new(Mutex::new(true)),
        }
    }

    #[cfg(test)]
    pub(crate) fn revoke_for_test(&self) {
        self.revoke();
    }
}

impl fmt::Debug for AccountRowToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AccountRowToken")
            .field("account_id", &self.account_id)
            .field("generation", &self.generation)
            .finish()
    }
}

impl PartialEq for AccountRowToken {
    fn eq(&self, other: &Self) -> bool {
        self.account_id == other.account_id
            && self.generation == other.generation
            && Arc::ptr_eq(&self.authorization, &other.authorization)
    }
}

impl Eq for AccountRowToken {}

impl SessionHealth {
    pub(super) fn can_launch(self) -> bool {
        matches!(self, Self::Valid | Self::CheckUnavailable)
    }
}

#[derive(Debug)]
pub(super) struct FetchedProfile {
    pub(super) id: u64,
    pub(super) username: String,
    pub(super) avatar_png: Option<Vec<u8>>,
}

#[derive(Debug)]
pub(crate) struct LinkedAccountData {
    pub(crate) record: StoredSessionAccount,
    pub(crate) avatar_png: Option<Vec<u8>>,
}

#[derive(Clone)]
pub(super) struct ConnectedAccount {
    pub(super) id: u64,
    pub(super) username: SharedString,
    pub(super) added_at_unix: i64,
    pub(super) avatar: Option<Arc<Image>>,
    pub(super) session_health: SessionHealth,
    generation: u64,
    authorization: Arc<Mutex<bool>>,
}

impl ConnectedAccount {
    fn from_stored(account: StoredSessionAccount, generation: u64) -> Self {
        Self {
            id: account.user_id,
            username: account.username.into(),
            added_at_unix: account.added_at_unix,
            avatar: None,
            session_health: SessionHealth::Checking,
            generation,
            authorization: Arc::new(Mutex::new(true)),
        }
    }

    fn from_linked(account: LinkedAccountData, generation: u64) -> Self {
        Self {
            id: account.record.user_id,
            username: account.record.username.into(),
            added_at_unix: account.record.added_at_unix,
            avatar: image_from_png(account.avatar_png),
            session_health: SessionHealth::Valid,
            generation,
            authorization: Arc::new(Mutex::new(true)),
        }
    }

    fn apply_profile(&mut self, profile: FetchedProfile) {
        self.username = profile.username.into();
        self.avatar = image_from_png(profile.avatar_png);
    }

    fn token(&self) -> AccountRowToken {
        AccountRowToken {
            account_id: self.id,
            generation: self.generation,
            authorization: self.authorization.clone(),
        }
    }

    fn stored_account(&self) -> StoredSessionAccount {
        StoredSessionAccount {
            user_id: self.id,
            username: self.username.to_string(),
            added_at_unix: self.added_at_unix,
        }
    }

    pub(super) fn formatted_added_at(&self) -> String {
        format_added_at(self.added_at_unix)
    }
}

fn image_from_png(bytes: Option<Vec<u8>>) -> Option<Arc<Image>> {
    bytes.map(|bytes| Arc::new(Image::from_bytes(ImageFormat::Png, bytes)))
}

pub(super) struct AccountStore {
    visible: Vec<ConnectedAccount>,
    instances: InstanceRegistry,
    next_generation: u64,
}

impl AccountStore {
    pub(super) fn new(accounts: Vec<StoredSessionAccount>) -> Self {
        let mut next_generation = 0_u64;
        let visible = accounts
            .into_iter()
            .map(|account| {
                next_generation = next_generation
                    .checked_add(1)
                    .expect("account row generation exhausted");
                ConnectedAccount::from_stored(account, next_generation)
            })
            .collect::<Vec<_>>();
        let instances = InstanceRegistry::with_accounts(visible.iter().map(|account| account.id));

        Self {
            visible,
            instances,
            next_generation,
        }
    }

    pub(super) fn visible(&self) -> &[ConnectedAccount] {
        &self.visible
    }

    pub(super) fn contains(&self, account_id: u64) -> bool {
        self.visible.iter().any(|account| account.id == account_id)
    }

    pub(super) fn add(&mut self, account: LinkedAccountData) -> bool {
        if self.contains(account.record.user_id) {
            return false;
        }

        let generation = self.advance_generation();
        let account = ConnectedAccount::from_linked(account, generation);
        self.instances.ensure_accounts([account.id]);
        self.visible.push(account);
        true
    }

    pub(super) fn remove(&mut self, account_id: u64) {
        if let Some(account) = self.visible.iter().find(|account| account.id == account_id) {
            account.token().revoke();
        }
        self.visible.retain(|account| account.id != account_id);
        self.instances.remove(account_id);
    }

    pub(super) fn begin_removal(&mut self, account_id: u64) -> bool {
        let Some(account) = self.visible.iter().find(|account| account.id == account_id) else {
            return false;
        };
        let mut authorization = account
            .authorization
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let was_authorized = *authorization;
        *authorization = false;
        was_authorized
    }

    pub(super) fn cancel_removal(&mut self, account_id: u64) -> bool {
        let Some(index) = self
            .visible
            .iter()
            .position(|account| account.id == account_id)
        else {
            return false;
        };
        let generation = self.advance_generation();
        let account = &mut self.visible[index];
        account.generation = generation;
        account.authorization = Arc::new(Mutex::new(true));
        true
    }

    pub(super) fn apply_profiles(&mut self, profiles: Vec<FetchedProfile>) {
        let mut profiles = profiles
            .into_iter()
            .map(|profile| (profile.id, profile))
            .collect::<HashMap<_, _>>();

        for account in &mut self.visible {
            if let Some(profile) = profiles.remove(&account.id) {
                account.apply_profile(profile);
            }
        }
    }

    pub(super) fn instance_phase(&self, account_id: u64) -> Option<InstancePhase> {
        self.instances.phase(account_id)
    }

    pub(super) fn apply_session_health(
        &mut self,
        token: &AccountRowToken,
        session_health: SessionHealth,
    ) -> bool {
        if !token.is_authorized() {
            return false;
        }
        let Some(account) = self.visible.iter_mut().find(|account| {
            account.id == token.account_id
                && account.generation == token.generation
                && Arc::ptr_eq(&account.authorization, &token.authorization)
        }) else {
            return false;
        };
        account.session_health = session_health;
        true
    }

    pub(super) fn session_check_token(&self, account_id: u64) -> Option<AccountRowToken> {
        self.visible
            .iter()
            .find(|account| account.id == account_id)
            .map(ConnectedAccount::token)
    }

    pub(super) fn reauthentication_request(
        &self,
        account_id: u64,
    ) -> Option<(AccountRowToken, StoredSessionAccount)> {
        let account = self.visible.iter().find(|account| {
            account.id == account_id
                && account.session_health == SessionHealth::LoginRequired
                && *account
                    .authorization
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
        })?;
        Some((account.token(), account.stored_account()))
    }

    pub(super) fn apply_reauthentication(
        &mut self,
        token: &AccountRowToken,
        account: LinkedAccountData,
    ) -> bool {
        if account.record.user_id != token.account_id || !token.is_authorized() {
            return false;
        }
        let Some(index) = self.visible.iter().position(|account| {
            account.id == token.account_id
                && account.generation == token.generation
                && Arc::ptr_eq(&account.authorization, &token.authorization)
        }) else {
            return false;
        };

        let generation = self.advance_generation();
        token.revoke();
        let row = &mut self.visible[index];
        row.username = account.record.username.into();
        row.session_health = SessionHealth::Valid;
        row.generation = generation;
        row.authorization = Arc::new(Mutex::new(true));
        true
    }

    pub(super) fn session_health(&self, account_id: u64) -> Option<SessionHealth> {
        self.visible
            .iter()
            .find(|account| account.id == account_id)
            .map(|account| account.session_health)
    }

    pub(super) fn begin_instance_transition(&mut self, account_id: u64) -> Option<TransitionToken> {
        self.instances.begin_primary_action(account_id)
    }

    pub(super) fn complete_instance_transition(&mut self, token: TransitionToken) -> bool {
        self.instances.complete(token)
    }

    pub(super) fn attach_client(
        &mut self,
        account_id: u64,
        client: crate::launcher::TrackedClient,
    ) {
        self.instances.attach_client(account_id, client);
    }

    pub(super) fn terminate_client(&mut self, account_id: u64) -> bool {
        self.instances.terminate_client(account_id)
    }

    pub(super) fn reap_exited_clients(&mut self) -> Vec<u64> {
        self.instances.reap_exited_clients()
    }

    pub(super) fn force_idle(&mut self, account_id: u64) -> bool {
        self.instances.force_idle(account_id)
    }

    pub(super) fn has_running_client(&self, account_id: u64) -> bool {
        self.instances.has_client(account_id)
    }

    fn advance_generation(&mut self) -> u64 {
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .expect("account row generation exhausted");
        self.next_generation
    }
}

fn format_added_at(timestamp: i64) -> String {
    let local_offset = UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC);
    format_added_at_with_offset(timestamp, local_offset)
}

fn format_added_at_with_offset(timestamp: i64, offset: UtcOffset) -> String {
    let Ok(date_time) = OffsetDateTime::from_unix_timestamp(timestamp) else {
        return "Unknown".into();
    };
    let date = date_time.to_offset(offset).date();
    let month = match date.month() {
        time::Month::January => "Jan",
        time::Month::February => "Feb",
        time::Month::March => "Mar",
        time::Month::April => "Apr",
        time::Month::May => "May",
        time::Month::June => "Jun",
        time::Month::July => "Jul",
        time::Month::August => "Aug",
        time::Month::September => "Sep",
        time::Month::October => "Oct",
        time::Month::November => "Nov",
        time::Month::December => "Dec",
    };
    format!("{:02} {month} {}", date.day(), date.year())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(id: u64, username: &str, added_at_unix: i64) -> StoredSessionAccount {
        StoredSessionAccount {
            user_id: id,
            username: username.into(),
            added_at_unix,
        }
    }

    fn profile(id: u64, username: &str) -> FetchedProfile {
        FetchedProfile {
            id,
            username: username.into(),
            avatar_png: None,
        }
    }

    fn visible_ids(store: &AccountStore) -> Vec<u64> {
        store.visible().iter().map(|account| account.id).collect()
    }

    #[test]
    fn stale_profile_refresh_cannot_resurrect_a_removed_account() {
        let mut store = AccountStore::new(vec![
            stored(42, "first", 1_700_000_000),
            stored(7, "second", 1_700_000_001),
        ]);
        store.remove(42);

        store.apply_profiles(vec![profile(42, "updated"), profile(7, "second")]);

        assert_eq!(visible_ids(&store), vec![7]);
    }

    #[test]
    fn profile_refresh_updates_only_existing_rows_without_reordering() {
        let mut store = AccountStore::new(vec![
            stored(42, "old-first", 1_700_000_000),
            stored(7, "old-second", 1_700_000_001),
        ]);

        store.apply_profiles(vec![profile(7, "new-second"), profile(42, "new-first")]);

        assert_eq!(visible_ids(&store), vec![42, 7]);
        assert_eq!(store.visible()[0].username, "new-first");
        assert_eq!(store.visible()[1].username, "new-second");
    }

    #[test]
    fn duplicate_link_does_not_duplicate_a_row() {
        let mut store = AccountStore::new(vec![stored(42, "existing", 1_700_000_000)]);

        assert!(!store.add(LinkedAccountData {
            record: stored(42, "duplicate", 1_700_000_001),
            avatar_png: None,
        }));
        assert_eq!(visible_ids(&store), vec![42]);
    }

    #[test]
    fn stored_accounts_start_checking_and_new_links_start_valid() {
        let store = AccountStore::new(vec![stored(42, "stored", 1_700_000_000)]);
        assert_eq!(store.session_health(42), Some(SessionHealth::Checking));

        let mut store = AccountStore::new(Vec::new());
        assert!(store.add(LinkedAccountData {
            record: stored(7, "linked", 1_700_000_001),
            avatar_png: None,
        }));
        assert_eq!(store.session_health(7), Some(SessionHealth::Valid));
    }

    #[test]
    fn launch_permission_fails_closed_while_checking_or_logged_out() {
        assert!(!SessionHealth::Checking.can_launch());
        assert!(SessionHealth::Valid.can_launch());
        assert!(!SessionHealth::LoginRequired.can_launch());
        assert!(SessionHealth::CheckUnavailable.can_launch());
        assert!(!SessionHealth::CredentialUnavailable.can_launch());
    }

    #[test]
    fn stale_session_check_cannot_update_a_relinked_row() {
        let mut store = AccountStore::new(vec![stored(42, "old", 1_700_000_000)]);
        let stale = store
            .session_check_token(42)
            .expect("stored account should have a check token");
        store.remove(42);
        assert!(store.add(LinkedAccountData {
            record: stored(42, "relinked", 1_800_000_000),
            avatar_png: None,
        }));

        assert!(!store.apply_session_health(&stale, SessionHealth::LoginRequired));
        assert_eq!(store.session_health(42), Some(SessionHealth::Valid));
    }

    #[test]
    fn reauthentication_request_requires_login_required() {
        let mut store = AccountStore::new(vec![stored(42, "stored", 1_700_000_000)]);
        assert!(store.reauthentication_request(42).is_none());

        store.apply_profiles(vec![profile(42, "current")]);
        let check = store.session_check_token(42).unwrap();
        assert!(store.apply_session_health(&check, SessionHealth::LoginRequired));

        let (token, account) = store.reauthentication_request(42).unwrap();
        assert_eq!(token, check);
        assert_eq!(account, stored(42, "current", 1_700_000_000));
    }

    #[test]
    fn pending_removal_revokes_reauthentication_tokens() {
        let mut store = AccountStore::new(vec![stored(42, "stored", 1_700_000_000)]);
        let check = store.session_check_token(42).unwrap();
        assert!(store.apply_session_health(&check, SessionHealth::LoginRequired));
        let (stale, _) = store.reauthentication_request(42).unwrap();

        assert!(store.begin_removal(42));
        assert!(!stale.is_authorized());
        assert!(store.reauthentication_request(42).is_none());

        assert!(store.cancel_removal(42));
        let (current, _) = store.reauthentication_request(42).unwrap();
        assert!(current.is_authorized());
        assert_ne!(current, stale);
    }

    #[test]
    fn successful_reauthentication_updates_only_the_session_row() {
        let mut store = AccountStore::new(vec![stored(42, "old", 1_700_000_000)]);
        let check = store.session_check_token(42).unwrap();
        assert!(store.apply_session_health(&check, SessionHealth::LoginRequired));
        let (token, _) = store.reauthentication_request(42).unwrap();
        let _ = store.begin_instance_transition(42).unwrap();

        assert!(store.apply_reauthentication(
            &token,
            LinkedAccountData {
                record: stored(42, "new", 1_800_000_000),
                avatar_png: Some(vec![1, 2, 3]),
            },
        ));

        let row = &store.visible()[0];
        assert_eq!(row.username, "new");
        assert_eq!(row.added_at_unix, 1_700_000_000);
        assert!(row.avatar.is_none());
        assert_eq!(row.session_health, SessionHealth::Valid);
        assert_eq!(store.instance_phase(42), Some(InstancePhase::Starting));
        assert_ne!(store.session_check_token(42), Some(token.clone()));
        assert!(!store.apply_session_health(&token, SessionHealth::LoginRequired));
    }

    #[test]
    fn reauthentication_rejects_the_wrong_account_or_generation() {
        let mut store = AccountStore::new(vec![stored(42, "old", 1_700_000_000)]);
        let check = store.session_check_token(42).unwrap();
        assert!(store.apply_session_health(&check, SessionHealth::LoginRequired));
        let (stale, _) = store.reauthentication_request(42).unwrap();

        assert!(!store.apply_reauthentication(
            &stale,
            LinkedAccountData {
                record: stored(7, "wrong", 1_800_000_000),
                avatar_png: None,
            },
        ));
        assert_eq!(store.visible()[0].username, "old");
        assert_eq!(store.session_check_token(42), Some(stale.clone()));

        store.remove(42);
        assert!(store.add(LinkedAccountData {
            record: stored(42, "relinked", 1_900_000_000),
            avatar_png: None,
        }));
        assert!(!store.apply_reauthentication(
            &stale,
            LinkedAccountData {
                record: stored(42, "late", 1_800_000_000),
                avatar_png: None,
            },
        ));
        assert_eq!(store.visible()[0].username, "relinked");
    }

    #[test]
    fn newly_linked_account_appends_with_an_idle_instance() {
        let mut store = AccountStore::new(vec![stored(42, "existing", 1_700_000_000)]);

        assert!(store.add(LinkedAccountData {
            record: stored(7, "new", 1_700_000_001),
            avatar_png: None,
        }));

        assert_eq!(visible_ids(&store), vec![42, 7]);
        assert_eq!(store.instance_phase(7), Some(InstancePhase::Idle));
    }

    #[test]
    fn profile_refresh_preserves_a_pending_instance_transition() {
        let mut store = AccountStore::new(vec![stored(42, "account", 1_700_000_000)]);
        let token = store
            .begin_instance_transition(42)
            .expect("idle account should begin starting");

        store.apply_profiles(vec![profile(42, "updated")]);

        assert_eq!(store.instance_phase(42), Some(InstancePhase::Starting));
        assert!(store.complete_instance_transition(token));
        assert_eq!(store.instance_phase(42), Some(InstancePhase::Running));
    }

    #[test]
    fn added_at_is_formatted_without_platform_locale_variance() {
        assert_eq!(
            format_added_at_with_offset(1_721_952_000, UtcOffset::UTC),
            "26 Jul 2024"
        );
        assert_eq!(
            format_added_at_with_offset(i64::MAX, UtcOffset::UTC),
            "Unknown"
        );
    }
}
