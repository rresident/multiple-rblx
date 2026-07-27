mod browser;

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{Arc, Mutex},
    thread,
};

use async_channel::{Receiver, Sender};

use crate::{
    accounts::{
        AccountRowToken, LinkedAccountData,
        service::{
            AccountServices, LinkAccountFailure, PendingAccountReauthentication,
            PreparedAccountLink, PreparedAccountReauthentication, ReauthenticateAccountFailure,
        },
    },
    security::StoredSessionAccount,
};

use browser::{LoginFailureKind, LoginOutcome, LoginSession};

#[derive(Debug)]
pub(crate) enum LinkEvent {
    BrowserReady,
    CheckingAccount,
    SavingAccount,
    Finished(Result<LinkResult, LinkFailure>),
}

#[derive(Clone, Debug)]
pub(crate) enum LinkRequest {
    Add,
    Reauthenticate {
        token: AccountRowToken,
        account: StoredSessionAccount,
    },
}

enum LinkWork {
    Add,
    Reauthenticate {
        token: AccountRowToken,
        pending: PendingAccountReauthentication,
    },
}

#[derive(Debug)]
pub(crate) enum LinkResult {
    Added(LinkedAccountData),
    Reauthenticated {
        token: AccountRowToken,
        account: LinkedAccountData,
    },
}

impl LinkResult {
    pub(crate) fn account_id(&self) -> u64 {
        match self {
            Self::Added(account) | Self::Reauthenticated { account, .. } => account.record.user_id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LinkFailure {
    Cancelled,
    SignInRejected,
    Network,
    Duplicate { username: String },
    WrongAccount { username: String },
    Conflict,
    SecureStorage,
    BrowserUnavailable,
    Browser,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommitState {
    Active,
    Adding,
    Reauthenticating,
    Cancelled,
    LinkedAdd(u64),
    LinkedReauthentication(u64),
    CleanupPending(u64),
    Accepted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinkCancelStatus {
    Cancelled,
    Finishing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GateCancel {
    Done,
    Cleanup(u64),
    Finishing,
    Completed,
}

struct CommitGate {
    state: Mutex<CommitState>,
}

impl CommitGate {
    fn new() -> Self {
        Self {
            state: Mutex::new(CommitState::Active),
        }
    }

    fn is_cancelled(&self) -> bool {
        matches!(
            *self.state.lock().unwrap_or_else(|error| error.into_inner()),
            CommitState::Cancelled
        )
    }

    fn commit_add(
        &self,
        services: &AccountServices,
        prepared: PreparedAccountLink,
    ) -> Result<LinkResult, LinkFailure> {
        {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if *state == CommitState::Cancelled {
                return Err(LinkFailure::Cancelled);
            }
            if *state != CommitState::Active {
                return Err(LinkFailure::Browser);
            }
            *state = CommitState::Adding;
        }

        let linked = match services.commit_link(prepared) {
            Ok(linked) => linked,
            Err(failure) => {
                let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                return if *state == CommitState::Cancelled {
                    Err(LinkFailure::Cancelled)
                } else {
                    Err(map_account_failure(failure))
                };
            }
        };
        let account_id = linked.record.user_id;

        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if *state == CommitState::Cancelled {
            *state = CommitState::CleanupPending(account_id);
            drop(state);
            if services.remove(account_id).is_err() {
                tracing::error!(
                    account_id,
                    "could not remove a session committed during cancellation"
                );
                self.cleanup_failed(account_id);
                return Err(LinkFailure::SecureStorage);
            }
            self.cleanup_succeeded(account_id);
            return Err(LinkFailure::Cancelled);
        }

        if *state != CommitState::Adding {
            return Err(LinkFailure::Browser);
        }
        *state = CommitState::LinkedAdd(account_id);
        Ok(LinkResult::Added(linked))
    }

    fn begin_reauthentication(&self, token: &AccountRowToken) -> Result<(), LinkFailure> {
        if !token.is_authorized() {
            return Err(LinkFailure::Conflict);
        }
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if *state == CommitState::Cancelled {
            return Err(LinkFailure::Cancelled);
        }
        if *state != CommitState::Active {
            return Err(LinkFailure::Browser);
        }
        if !token.is_authorized() {
            return Err(LinkFailure::Conflict);
        }
        *state = CommitState::Reauthenticating;
        Ok(())
    }

    fn commit_reauthentication(
        &self,
        services: &AccountServices,
        token: AccountRowToken,
        prepared: PreparedAccountReauthentication,
    ) -> Result<LinkResult, LinkFailure> {
        let Some(authorization) = token.hold_authorization() else {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            if *state == CommitState::Reauthenticating {
                *state = CommitState::Cancelled;
            }
            return Err(LinkFailure::Conflict);
        };
        let linked = match services.commit_reauthentication(prepared) {
            Ok(linked) => linked,
            Err(failure) => {
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                if *state == CommitState::Reauthenticating {
                    *state = CommitState::Cancelled;
                }
                return Err(map_reauthentication_failure(failure));
            }
        };
        drop(authorization);
        let account_id = linked.record.user_id;

        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if *state != CommitState::Reauthenticating {
            return Err(LinkFailure::Browser);
        }
        *state = CommitState::LinkedReauthentication(account_id);
        Ok(LinkResult::Reauthenticated {
            token,
            account: linked,
        })
    }

    fn accept(&self, account_id: u64) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if !matches!(
            *state,
            CommitState::LinkedAdd(id) | CommitState::LinkedReauthentication(id)
                if id == account_id
        ) {
            return false;
        }

        *state = CommitState::Accepted;
        true
    }

    fn cancel(&self) -> GateCancel {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        match *state {
            CommitState::Active | CommitState::Adding => {
                *state = CommitState::Cancelled;
                GateCancel::Done
            }
            CommitState::LinkedAdd(account_id) => {
                *state = CommitState::Cancelled;
                GateCancel::Cleanup(account_id)
            }
            CommitState::Reauthenticating | CommitState::LinkedReauthentication(_) => {
                GateCancel::Finishing
            }
            CommitState::CleanupPending(account_id) => GateCancel::Cleanup(account_id),
            CommitState::Cancelled => GateCancel::Done,
            CommitState::Accepted => GateCancel::Completed,
        }
    }

    fn cleanup_succeeded(&self, account_id: u64) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if *state == CommitState::CleanupPending(account_id) {
            *state = CommitState::Cancelled;
        }
    }

    fn cleanup_failed(&self, account_id: u64) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        *state = CommitState::CleanupPending(account_id);
    }

    fn worker_panicked(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if matches!(
            *state,
            CommitState::Reauthenticating | CommitState::LinkedReauthentication(_)
        ) {
            *state = CommitState::Cancelled;
        }
    }
}

pub(crate) struct LinkAttempt {
    browser: LoginSession,
    ready: Receiver<()>,
    events: Receiver<LinkEvent>,
    gate: Arc<CommitGate>,
    services: Arc<AccountServices>,
}

impl LinkAttempt {
    pub(crate) fn start(
        services: Arc<AccountServices>,
        request: LinkRequest,
    ) -> Result<Self, LinkFailure> {
        let work = prepare_link_work(&services, request)?;
        tracing::info!("starting isolated Roblox sign-in");
        let mut browser = browser::start().map_err(|error| {
            tracing::error!(reason = %error, "Roblox sign-in worker could not start");
            LinkFailure::BrowserUnavailable
        })?;
        let ready = browser.ready();
        let browser_result = browser
            .take_result()
            .ok_or(LinkFailure::BrowserUnavailable)?;
        let (event_sender, events) = async_channel::bounded(3);
        let gate = Arc::new(CommitGate::new());

        let services_for_worker = services.clone();
        let gate_for_worker = gate.clone();
        thread::Builder::new()
            .name("roblox-account-link".into())
            .spawn(move || {
                let panic_sender = event_sender.clone();
                let panic_gate = gate_for_worker.clone();
                let guarded = catch_unwind(AssertUnwindSafe(|| {
                    run_link_worker(
                        browser_result,
                        event_sender,
                        services_for_worker,
                        gate_for_worker,
                        work,
                    );
                }));
                if guarded.is_err() {
                    tracing::error!("Roblox account-link worker panicked");
                    panic_gate.worker_panicked();
                    let _ =
                        panic_sender.send_blocking(LinkEvent::Finished(Err(LinkFailure::Browser)));
                }
            })
            .map_err(|_| LinkFailure::BrowserUnavailable)?;

        Ok(Self {
            browser,
            ready,
            events,
            gate,
            services,
        })
    }

    pub(crate) fn events(&self) -> Receiver<LinkEvent> {
        self.events.clone()
    }

    pub(crate) fn ready_events(&self) -> Receiver<()> {
        self.ready.clone()
    }

    pub(crate) fn accept(&self, account_id: u64) -> bool {
        self.gate.accept(account_id)
    }

    pub(crate) fn cancel(&self) -> Result<LinkCancelStatus, LinkFailure> {
        let browser_cancelled = self.browser.cancel();
        let gate_cancel = self.gate.cancel();
        if browser_cancelled && gate_cancel != GateCancel::Completed {
            tracing::info!("cancelling Roblox account linking");
        }
        match gate_cancel {
            GateCancel::Done | GateCancel::Completed => Ok(LinkCancelStatus::Cancelled),
            GateCancel::Finishing => Ok(LinkCancelStatus::Finishing),
            GateCancel::Cleanup(account_id) => {
                if self.services.remove(account_id).is_err() {
                    tracing::error!(
                        account_id,
                        "could not remove a session committed during cancellation"
                    );
                    self.gate.cleanup_failed(account_id);
                    return Err(LinkFailure::SecureStorage);
                }
                self.gate.cleanup_succeeded(account_id);
                Ok(LinkCancelStatus::Cancelled)
            }
        }
    }
}

impl Drop for LinkAttempt {
    fn drop(&mut self) {
        let _ = self.cancel();
    }
}

fn run_link_worker(
    browser_result: Receiver<LoginOutcome>,
    events: Sender<LinkEvent>,
    services: Arc<AccountServices>,
    gate: Arc<CommitGate>,
    work: LinkWork,
) {
    let outcome = browser_result
        .recv_blocking()
        .unwrap_or(LoginOutcome::Cancelled);
    let terminal = match outcome {
        LoginOutcome::Cookie(session) => {
            tracing::debug!("Roblox sign-in completed; cookie captured and redacted");
            if gate.is_cancelled() {
                Err(LinkFailure::Cancelled)
            } else {
                let _ = events.try_send(LinkEvent::CheckingAccount);
                match work {
                    LinkWork::Add => match services.prepare_link(session) {
                        Ok(prepared) => gate.commit_add(&services, prepared),
                        Err(failure) => Err(map_account_failure(failure)),
                    },
                    LinkWork::Reauthenticate { token, pending } => {
                        if !token.is_authorized() {
                            Err(LinkFailure::Conflict)
                        } else {
                            match services.prepare_reauthentication(pending, session) {
                                Ok(prepared) => {
                                    gate.begin_reauthentication(&token).and_then(|()| {
                                        let _ = events.send_blocking(LinkEvent::SavingAccount);
                                        gate.commit_reauthentication(&services, token, prepared)
                                    })
                                }
                                Err(failure) => Err(map_reauthentication_failure(failure)),
                            }
                        }
                    }
                }
            }
        }
        LoginOutcome::Cancelled => {
            tracing::info!("Roblox sign-in window cancelled");
            Err(LinkFailure::Cancelled)
        }
        LoginOutcome::Failed(failure) => {
            tracing::error!(
                category = ?failure.kind,
                reason = %failure.message,
                "Roblox sign-in window failed"
            );
            Err(match failure.kind {
                LoginFailureKind::WebViewUnavailable => LinkFailure::BrowserUnavailable,
                LoginFailureKind::Native => LinkFailure::Browser,
            })
        }
    };

    let _ = events.send_blocking(LinkEvent::Finished(terminal));
}

fn prepare_link_work(
    services: &AccountServices,
    request: LinkRequest,
) -> Result<LinkWork, LinkFailure> {
    match request {
        LinkRequest::Add => Ok(LinkWork::Add),
        LinkRequest::Reauthenticate { token, account } => {
            if !token.is_authorized() {
                return Err(LinkFailure::Conflict);
            }
            let pending = services
                .begin_reauthentication(account)
                .map_err(map_reauthentication_failure)?;
            if !token.is_authorized() {
                return Err(LinkFailure::Conflict);
            }
            Ok(LinkWork::Reauthenticate { token, pending })
        }
    }
}

fn map_account_failure(failure: LinkAccountFailure) -> LinkFailure {
    match failure {
        LinkAccountFailure::Rejected => LinkFailure::SignInRejected,
        LinkAccountFailure::Unavailable => LinkFailure::Network,
        LinkAccountFailure::Duplicate { username } => LinkFailure::Duplicate { username },
        LinkAccountFailure::Storage => LinkFailure::SecureStorage,
    }
}

fn map_reauthentication_failure(failure: ReauthenticateAccountFailure) -> LinkFailure {
    match failure {
        ReauthenticateAccountFailure::Rejected => LinkFailure::SignInRejected,
        ReauthenticateAccountFailure::Unavailable => LinkFailure::Network,
        ReauthenticateAccountFailure::WrongAccount { username } => {
            LinkFailure::WrongAccount { username }
        }
        ReauthenticateAccountFailure::Conflict => LinkFailure::Conflict,
        ReauthenticateAccountFailure::Storage => LinkFailure::SecureStorage,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        sync::{
            Arc, Condvar, Mutex,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::Duration,
    };

    use anyhow::Result as AnyResult;
    use secrecy::{ExposeSecret as _, SecretString};

    use super::{CommitGate, CommitState, GateCancel, LinkFailure};
    use crate::{
        accounts::{
            AccountRowToken,
            service::{AccountServices, PreparedAccountLink, PreparedAccountReauthentication},
        },
        security::{
            ReplaceSessionResult, SessionBaseline, SessionVault, StoreSessionResult,
            StoredSessionAccount,
        },
    };

    #[derive(Default)]
    struct BlockingVault {
        state: Mutex<BlockingVaultState>,
        changed: Condvar,
    }

    #[derive(Default)]
    struct BlockingVaultState {
        store_started: bool,
        release_store: bool,
        replace_started: bool,
        block_replace: bool,
        release_replace: bool,
        accounts: Vec<(StoredSessionAccount, SecretString)>,
        delete_count: usize,
    }

    impl BlockingVault {
        fn wait_until_store_starts(&self) {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let (state, _) = self
                .changed
                .wait_timeout_while(state, Duration::from_secs(5), |state| !state.store_started)
                .unwrap_or_else(|error| error.into_inner());
            assert!(
                state.store_started,
                "commit should reach the blocking vault"
            );
        }

        fn release_store(&self) {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.release_store = true;
            self.changed.notify_all();
        }

        fn block_replacement(&self) {
            self.state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .block_replace = true;
        }

        fn wait_until_replacement_starts(&self) {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let (state, _) = self
                .changed
                .wait_timeout_while(state, Duration::from_secs(5), |state| {
                    !state.replace_started
                })
                .unwrap_or_else(|error| error.into_inner());
            assert!(state.replace_started, "replacement should reach the vault");
        }

        fn release_replacement(&self) {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.release_replace = true;
            self.changed.notify_all();
        }

        fn account_count(&self) -> usize {
            self.state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .accounts
                .len()
        }

        fn delete_count(&self) -> usize {
            self.state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .delete_count
        }
    }

    #[derive(Default)]
    struct PanickingReplacementVault {
        replaced: AtomicBool,
    }

    impl SessionVault for PanickingReplacementVault {
        fn list(&self) -> AnyResult<Vec<StoredSessionAccount>> {
            Ok(Vec::new())
        }

        fn load_session(&self, _: u64) -> AnyResult<Option<SecretString>> {
            Ok(None)
        }

        fn snapshot(&self, user_id: u64) -> AnyResult<SessionBaseline> {
            Ok(SessionBaseline::missing(user_id))
        }

        fn store_if_absent(
            &self,
            _: &StoredSessionAccount,
            _: &SecretString,
        ) -> AnyResult<StoreSessionResult> {
            unreachable!()
        }

        fn replace_if_unchanged(
            &self,
            _: &SessionBaseline,
            _: &StoredSessionAccount,
            _: &SecretString,
        ) -> AnyResult<ReplaceSessionResult> {
            self.replaced.store(true, Ordering::Release);
            panic!("replacement vault panic")
        }

        fn delete(&self, _: u64) -> AnyResult<()> {
            Ok(())
        }
    }

    impl SessionVault for BlockingVault {
        fn list(&self) -> AnyResult<Vec<StoredSessionAccount>> {
            Ok(self
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .accounts
                .iter()
                .map(|(account, _)| account.clone())
                .collect())
        }

        fn load_session(&self, user_id: u64) -> AnyResult<Option<SecretString>> {
            Ok(self
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .accounts
                .iter()
                .find(|(account, _)| account.user_id == user_id)
                .map(|(_, session)| SecretString::from(session.expose_secret().to_owned())))
        }

        fn snapshot(&self, user_id: u64) -> AnyResult<SessionBaseline> {
            let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            Ok(state
                .accounts
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
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.store_started = true;
            self.changed.notify_all();
            while !state.release_store {
                state = self
                    .changed
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }

            if let Some((existing, _)) = state
                .accounts
                .iter()
                .find(|(existing, _)| existing.user_id == account.user_id)
            {
                return Ok(StoreSessionResult::AlreadyExists(existing.clone()));
            }
            state.accounts.push((
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
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state.replace_started = true;
            self.changed.notify_all();
            while state.block_replace && !state.release_replace {
                state = self
                    .changed
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
            let index = state
                .accounts
                .iter()
                .position(|(current, _)| current.user_id == account.user_id);
            let current = index.map(|index| {
                let (account, session) = &state.accounts[index];
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
                state.accounts[index] = replacement;
            } else {
                state.accounts.push(replacement);
            }
            Ok(ReplaceSessionResult::Replaced)
        }

        fn delete(&self, user_id: u64) -> AnyResult<()> {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            state
                .accounts
                .retain(|(account, _)| account.user_id != user_id);
            state.delete_count += 1;
            Ok(())
        }
    }

    #[test]
    fn an_unaccepted_link_is_cancelled_at_the_commit_gate() {
        let gate = CommitGate::new();
        assert_eq!(gate.cancel(), GateCancel::Done);
        assert!(gate.is_cancelled());
        assert!(!gate.accept(42));
    }

    #[test]
    fn accepting_requires_the_exact_linked_account() {
        let gate = CommitGate {
            state: Mutex::new(CommitState::LinkedAdd(42)),
        };
        assert!(!gate.accept(7));
        assert!(gate.accept(42));
        assert_eq!(gate.cancel(), GateCancel::Completed);
    }

    #[test]
    fn reauthentication_cannot_be_cancelled_during_replacement() {
        let gate = CommitGate {
            state: Mutex::new(CommitState::Reauthenticating),
        };
        assert_eq!(gate.cancel(), GateCancel::Finishing);
        assert!(!gate.is_cancelled());

        *gate.state.lock().unwrap_or_else(|error| error.into_inner()) =
            CommitState::LinkedReauthentication(42);
        assert_eq!(gate.cancel(), GateCancel::Finishing);
        assert!(gate.accept(42));
    }

    #[test]
    fn cancelling_a_blocked_commit_is_nonblocking_and_cleans_the_late_write() {
        let vault = Arc::new(BlockingVault::default());
        let services = AccountServices::with_vault_for_test(vault.clone());
        let prepared = PreparedAccountLink::for_test(
            StoredSessionAccount {
                user_id: 42,
                username: "account_name".into(),
                added_at_unix: 1_700_000_000,
            },
            SecretString::from("unit-test-session".to_owned()),
        );
        let gate = Arc::new(CommitGate::new());
        let commit_gate = gate.clone();
        let commit_worker = std::thread::spawn(move || commit_gate.commit_add(&services, prepared));

        vault.wait_until_store_starts();
        assert_eq!(
            *gate.state.lock().unwrap_or_else(|error| error.into_inner()),
            CommitState::Adding
        );

        let cancel_gate = gate.clone();
        let (cancel_sender, cancel_receiver) = mpsc::channel();
        let cancel_worker = std::thread::spawn(move || {
            let _ = cancel_sender.send(cancel_gate.cancel());
        });
        let cancellation = cancel_receiver.recv_timeout(Duration::from_secs(1));

        vault.release_store();
        let commit_result = commit_worker
            .join()
            .expect("commit worker should not panic");
        cancel_worker
            .join()
            .expect("cancel worker should not panic");

        assert_eq!(
            cancellation.expect("cancellation must not wait for secure storage"),
            GateCancel::Done
        );
        assert!(matches!(commit_result, Err(LinkFailure::Cancelled)));
        assert_eq!(vault.account_count(), 0);
        assert_eq!(vault.delete_count(), 1);
        assert_eq!(
            *gate.state.lock().unwrap_or_else(|error| error.into_inner()),
            CommitState::Cancelled
        );
    }

    #[test]
    fn panicking_replacement_leaves_the_failure_dialog_closable() {
        let vault = Arc::new(PanickingReplacementVault::default());
        let services = AccountServices::with_vault_for_test(vault.clone());
        let record = StoredSessionAccount {
            user_id: 42,
            username: "account_name".into(),
            added_at_unix: 1_700_000_000,
        };
        let prepared = PreparedAccountReauthentication::for_test(
            SessionBaseline::present(record.clone(), SecretString::from("old-session".to_owned())),
            record,
            SecretString::from("new-session".to_owned()),
        );
        let gate = CommitGate::new();
        let token = AccountRowToken::for_test(42);
        gate.begin_reauthentication(&token)
            .expect("reauthentication should begin");

        let result = catch_unwind(AssertUnwindSafe(|| {
            gate.commit_reauthentication(&services, token, prepared)
        }));

        assert!(result.is_err());
        assert!(vault.replaced.load(Ordering::Acquire));
        gate.worker_panicked();
        assert_eq!(
            *gate.state.lock().unwrap_or_else(|error| error.into_inner()),
            CommitState::Cancelled
        );
        assert_eq!(gate.cancel(), GateCancel::Done);
        assert_ne!(gate.cancel(), GateCancel::Finishing);
    }

    #[test]
    fn revoked_row_never_reaches_session_replacement() {
        let vault = Arc::new(PanickingReplacementVault::default());
        let services = AccountServices::with_vault_for_test(vault.clone());
        let record = StoredSessionAccount {
            user_id: 42,
            username: "account_name".into(),
            added_at_unix: 1_700_000_000,
        };
        let prepared = PreparedAccountReauthentication::for_test(
            SessionBaseline::present(record.clone(), SecretString::from("old-session".to_owned())),
            record,
            SecretString::from("new-session".to_owned()),
        );
        let gate = CommitGate::new();
        let token = AccountRowToken::for_test(42);
        gate.begin_reauthentication(&token)
            .expect("reauthentication should begin");
        token.revoke_for_test();

        assert!(matches!(
            gate.commit_reauthentication(&services, token, prepared),
            Err(LinkFailure::Conflict)
        ));
        assert!(!vault.replaced.load(Ordering::Acquire));
        assert_eq!(
            *gate.state.lock().unwrap_or_else(|error| error.into_inner()),
            CommitState::Cancelled
        );
    }

    #[test]
    fn removal_waits_for_replacement_then_deletes_the_result() {
        let vault = Arc::new(BlockingVault::default());
        vault.block_replacement();
        let services = AccountServices::with_vault_for_test(vault.clone());
        let record = StoredSessionAccount {
            user_id: 42,
            username: "account_name".into(),
            added_at_unix: 1_700_000_000,
        };
        let prepared = PreparedAccountReauthentication::for_test(
            SessionBaseline::missing(42),
            record,
            SecretString::from("new-session".to_owned()),
        );
        let gate = Arc::new(CommitGate::new());
        let token = AccountRowToken::for_test(42);
        gate.begin_reauthentication(&token)
            .expect("reauthentication should begin");

        let commit_gate = gate.clone();
        let commit_services = services.clone();
        let commit_token = token.clone();
        let commit = std::thread::spawn(move || {
            commit_gate.commit_reauthentication(&commit_services, commit_token, prepared)
        });
        vault.wait_until_replacement_starts();

        let removal_services = services.clone();
        let removal_token = token.clone();
        let (started_sender, started_receiver) = mpsc::channel();
        let (done_sender, done_receiver) = mpsc::channel();
        let removal = std::thread::spawn(move || {
            started_sender.send(()).expect("signalling removal");
            removal_token.revoke_for_test();
            removal_services.remove(42).expect("removing account");
            done_sender.send(()).expect("signalling removal completion");
        });
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("removal should start");
        assert!(
            done_receiver
                .recv_timeout(Duration::from_millis(100))
                .is_err()
        );

        vault.release_replacement();
        assert!(commit.join().expect("replacement should not panic").is_ok());
        removal.join().expect("removal should not panic");
        done_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("removal should finish");

        assert!(!token.is_authorized());
        assert_eq!(vault.account_count(), 0);
        assert_eq!(vault.delete_count(), 1);
    }
}
