use std::{collections::HashMap, marker::PhantomData, time::Duration};

use crate::launcher::TrackedClient;

pub(super) const CLIENT_POLL_INTERVAL: Duration = Duration::from_millis(1_200);

pub(super) const TRANSITION_DELAY: Duration = Duration::from_millis(600);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum InstancePhase {
    #[default]
    Idle,
    Starting,
    Running,
    Stopping,
}

#[derive(Debug)]
struct Idle;

#[derive(Debug)]
struct Starting;

#[derive(Debug)]
struct Running;

#[derive(Debug)]
struct Stopping;

#[derive(Debug)]
struct Instance<State> {
    account_id: u64,
    generation: u64,
    state: PhantomData<State>,
}

impl Instance<Idle> {
    fn new(account_id: u64) -> Self {
        Self {
            account_id,
            generation: 0,
            state: PhantomData,
        }
    }

    fn start(self) -> (Instance<Starting>, TransitionToken) {
        let generation = self
            .generation
            .checked_add(1)
            .expect("instance transition generation exhausted");
        let token = TransitionToken {
            account_id: self.account_id,
            generation,
        };

        (
            Instance {
                account_id: self.account_id,
                generation,
                state: PhantomData,
            },
            token,
        )
    }
}

impl Instance<Starting> {
    fn finish_starting(self) -> Instance<Running> {
        Instance {
            account_id: self.account_id,
            generation: self.generation,
            state: PhantomData,
        }
    }

    fn abandon(self) -> Instance<Idle> {
        Instance {
            account_id: self.account_id,
            generation: self.generation.saturating_add(1),
            state: PhantomData,
        }
    }
}

impl Instance<Running> {
    fn abandon(self) -> Instance<Idle> {
        Instance {
            account_id: self.account_id,
            generation: self.generation.saturating_add(1),
            state: PhantomData,
        }
    }

    fn stop(self) -> (Instance<Stopping>, TransitionToken) {
        let generation = self
            .generation
            .checked_add(1)
            .expect("instance transition generation exhausted");
        let token = TransitionToken {
            account_id: self.account_id,
            generation,
        };

        (
            Instance {
                account_id: self.account_id,
                generation,
                state: PhantomData,
            },
            token,
        )
    }
}

impl Instance<Stopping> {
    fn finish_stopping(self) -> Instance<Idle> {
        Instance {
            account_id: self.account_id,
            generation: self.generation,
            state: PhantomData,
        }
    }
}

#[must_use = "a transition token must be scheduled or deliberately discarded"]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TransitionToken {
    account_id: u64,
    generation: u64,
}

#[derive(Debug)]
enum ErasedInstance {
    Idle(Instance<Idle>),
    Starting(Instance<Starting>),
    Running(Instance<Running>),
    Stopping(Instance<Stopping>),
}

impl ErasedInstance {
    fn phase(&self) -> InstancePhase {
        match self {
            Self::Idle(_) => InstancePhase::Idle,
            Self::Starting(_) => InstancePhase::Starting,
            Self::Running(_) => InstancePhase::Running,
            Self::Stopping(_) => InstancePhase::Stopping,
        }
    }
}

#[derive(Default)]
pub(super) struct InstanceRegistry {
    instances: HashMap<u64, ErasedInstance>,
    clients: HashMap<u64, TrackedClient>,
}

impl InstanceRegistry {
    pub(super) fn with_accounts(account_ids: impl IntoIterator<Item = u64>) -> Self {
        let mut registry = Self::default();
        registry.ensure_accounts(account_ids);
        registry
    }

    pub(super) fn ensure_accounts(&mut self, account_ids: impl IntoIterator<Item = u64>) {
        for account_id in account_ids {
            self.instances
                .entry(account_id)
                .or_insert_with(|| ErasedInstance::Idle(Instance::new(account_id)));
        }
    }

    pub(super) fn phase(&self, account_id: u64) -> Option<InstancePhase> {
        self.instances.get(&account_id).map(ErasedInstance::phase)
    }

    pub(super) fn begin_primary_action(&mut self, account_id: u64) -> Option<TransitionToken> {
        let current = self.instances.remove(&account_id)?;

        match current {
            ErasedInstance::Idle(instance) => {
                let (starting, token) = instance.start();
                self.instances
                    .insert(account_id, ErasedInstance::Starting(starting));
                Some(token)
            }
            ErasedInstance::Running(instance) => {
                let (stopping, token) = instance.stop();
                self.instances
                    .insert(account_id, ErasedInstance::Stopping(stopping));
                Some(token)
            }
            transitional => {
                self.instances.insert(account_id, transitional);
                None
            }
        }
    }

    pub(super) fn complete(&mut self, token: TransitionToken) -> bool {
        let Some(current) = self.instances.remove(&token.account_id) else {
            return false;
        };

        match current {
            ErasedInstance::Starting(instance) if instance.generation == token.generation => {
                self.instances.insert(
                    token.account_id,
                    ErasedInstance::Running(instance.finish_starting()),
                );
                true
            }
            ErasedInstance::Stopping(instance) if instance.generation == token.generation => {
                self.instances.insert(
                    token.account_id,
                    ErasedInstance::Idle(instance.finish_stopping()),
                );
                true
            }
            current => {
                self.instances.insert(token.account_id, current);
                false
            }
        }
    }

    pub(super) fn remove(&mut self, account_id: u64) {
        self.instances.remove(&account_id);
        self.clients.remove(&account_id);
    }

    pub(super) fn attach_client(&mut self, account_id: u64, client: TrackedClient) {
        if let Some(previous) = self.clients.insert(account_id, client) {
            drop(previous);
        }
    }

    pub(super) fn terminate_client(&mut self, account_id: u64) -> bool {
        let Some(client) = self.clients.remove(&account_id) else {
            return false;
        };
        client.terminate();
        true
    }

    pub(super) fn reap_exited_clients(&mut self) -> Vec<u64> {
        let mut exited = Vec::new();

        self.clients.retain(|account_id, client| {
            if client.has_exited() {
                tracing::info!(account_id, pid = client.pid(), "Roblox client exited");
                exited.push(*account_id);
                return false;
            }
            true
        });

        exited
    }

    pub(super) fn force_idle(&mut self, account_id: u64) -> bool {
        let Some(current) = self.instances.remove(&account_id) else {
            return false;
        };

        let (replacement, changed) = match current {
            ErasedInstance::Idle(instance) => (ErasedInstance::Idle(instance), false),
            ErasedInstance::Starting(instance) => (ErasedInstance::Idle(instance.abandon()), true),
            ErasedInstance::Running(instance) => (ErasedInstance::Idle(instance.abandon()), true),
            ErasedInstance::Stopping(instance) => {
                (ErasedInstance::Idle(instance.finish_stopping()), true)
            }
        };

        self.instances.insert(account_id, replacement);
        changed
    }

    pub(super) fn has_client(&self, account_id: u64) -> bool {
        self.clients.contains_key(&account_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typestates_expose_only_valid_transition_methods() {
        let idle = Instance::<Idle>::new(42);
        let (starting, start_token) = idle.start();
        assert_eq!(start_token.account_id, 42);

        let running = starting.finish_starting();
        let (stopping, stop_token) = running.stop();
        assert_eq!(stop_token.account_id, 42);

        let idle = stopping.finish_stopping();
        assert_eq!(idle.account_id, 42);
    }

    #[test]
    fn primary_action_walks_the_complete_lifecycle() {
        let mut registry = InstanceRegistry::with_accounts([42]);
        assert_eq!(registry.phase(42), Some(InstancePhase::Idle));

        let start = registry
            .begin_primary_action(42)
            .expect("idle instance should begin starting");
        assert_eq!(registry.phase(42), Some(InstancePhase::Starting));
        assert!(registry.complete(start));
        assert_eq!(registry.phase(42), Some(InstancePhase::Running));

        let stop = registry
            .begin_primary_action(42)
            .expect("running instance should begin stopping");
        assert_eq!(registry.phase(42), Some(InstancePhase::Stopping));
        assert!(registry.complete(stop));
        assert_eq!(registry.phase(42), Some(InstancePhase::Idle));
    }

    #[test]
    fn transitional_phases_reject_reentry() {
        let mut registry = InstanceRegistry::with_accounts([42]);
        let start = registry.begin_primary_action(42).unwrap();

        assert_eq!(registry.begin_primary_action(42), None);
        assert!(registry.complete(start));

        let stop = registry.begin_primary_action(42).unwrap();
        assert_eq!(registry.begin_primary_action(42), None);
        assert!(registry.complete(stop));
    }

    #[test]
    fn duplicate_and_stale_tokens_are_ignored() {
        let mut registry = InstanceRegistry::with_accounts([42]);
        let first_start = registry.begin_primary_action(42).unwrap();
        assert!(registry.complete(first_start));
        assert!(!registry.complete(first_start));

        let stop = registry.begin_primary_action(42).unwrap();
        assert!(registry.complete(stop));
        let second_start = registry.begin_primary_action(42).unwrap();

        assert!(!registry.complete(first_start));
        assert_eq!(registry.phase(42), Some(InstancePhase::Starting));
        assert!(registry.complete(second_start));
    }

    #[test]
    fn account_lifecycles_are_independent() {
        let mut registry = InstanceRegistry::with_accounts([42, 7]);
        let start = registry.begin_primary_action(42).unwrap();

        assert_eq!(registry.phase(42), Some(InstancePhase::Starting));
        assert_eq!(registry.phase(7), Some(InstancePhase::Idle));
        assert!(registry.complete(start));
        assert_eq!(registry.phase(42), Some(InstancePhase::Running));
        assert_eq!(registry.phase(7), Some(InstancePhase::Idle));
    }

    #[test]
    fn a_temporarily_hidden_account_keeps_its_pending_transition() {
        let mut registry = InstanceRegistry::with_accounts([42]);
        let start = registry.begin_primary_action(42).unwrap();

        registry.ensure_accounts([]);
        registry.ensure_accounts([42]);

        assert_eq!(registry.phase(42), Some(InstancePhase::Starting));
        assert_eq!(registry.begin_primary_action(42), None);
        assert!(registry.complete(start));
        assert_eq!(registry.phase(42), Some(InstancePhase::Running));
    }
}
