use std::{
    error::Error,
    fmt, io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};

use async_channel::{Receiver, Sender};
use secrecy::SecretString;

#[cfg(target_os = "windows")]
mod platform;

pub(super) fn start() -> Result<LoginSession, LoginStartError> {
    #[cfg(not(target_os = "windows"))]
    {
        return Err(LoginStartError::UnsupportedPlatform);
    }

    #[cfg(target_os = "windows")]
    {
        let (sender, receiver) = async_channel::bounded(1);
        let (ready_sender, ready) = async_channel::bounded(1);
        let completion = Arc::new(Completion::new(sender));
        let cancellation = LoginCancellation {
            state: Arc::new(CancellationState::default()),
        };

        platform::spawn_login_thread(completion, cancellation.state.clone(), ready_sender)
            .map_err(LoginStartError::WorkerSpawn)?;

        Ok(LoginSession {
            receiver: Some(receiver),
            ready,
            cancellation,
        })
    }
}

pub(super) struct LoginSession {
    receiver: Option<Receiver<LoginOutcome>>,
    ready: Receiver<()>,
    cancellation: LoginCancellation,
}

impl LoginSession {
    pub(super) fn ready(&self) -> Receiver<()> {
        self.ready.clone()
    }

    pub(super) fn cancel(&self) -> bool {
        self.cancellation.cancel()
    }

    pub(super) fn take_result(&mut self) -> Option<Receiver<LoginOutcome>> {
        self.receiver.take()
    }
}

impl Drop for LoginSession {
    fn drop(&mut self) {
        let _ = self.cancellation.cancel();
    }
}

#[derive(Clone)]
pub(super) struct LoginCancellation {
    state: Arc<CancellationState>,
}

impl LoginCancellation {
    pub(super) fn cancel(&self) -> bool {
        if self.state.requested.swap(true, Ordering::AcqRel) {
            return false;
        }

        #[cfg(target_os = "windows")]
        platform::wake_login_thread(&self.state);
        true
    }
}

pub(super) enum LoginOutcome {
    Cookie(SecretString),
    Cancelled,
    Failed(LoginFailure),
}

impl fmt::Debug for LoginOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cookie(_) => formatter.write_str("Cookie([REDACTED])"),
            Self::Cancelled => formatter.write_str("Cancelled"),
            Self::Failed(failure) => formatter.debug_tuple("Failed").field(failure).finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LoginFailureKind {
    WebViewUnavailable,
    Native,
}

#[derive(Debug)]
pub(super) struct LoginFailure {
    pub(super) kind: LoginFailureKind,
    pub(super) message: String,
}

#[derive(Debug)]
pub(super) enum LoginStartError {
    WorkerSpawn(io::Error),
    #[cfg(not(target_os = "windows"))]
    UnsupportedPlatform,
}

impl fmt::Display for LoginStartError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WorkerSpawn(error) => write!(formatter, "could not start login worker: {error}"),
            #[cfg(not(target_os = "windows"))]
            Self::UnsupportedPlatform => {
                formatter.write_str("the Roblox login browser is available only on Windows")
            }
        }
    }
}

impl Error for LoginStartError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::WorkerSpawn(error) => Some(error),
            #[cfg(not(target_os = "windows"))]
            Self::UnsupportedPlatform => None,
        }
    }
}

struct CancellationState {
    requested: AtomicBool,
    thread_id: Mutex<Option<u32>>,
    message_token: usize,
}

impl Default for CancellationState {
    fn default() -> Self {
        static NEXT_MESSAGE_TOKEN: AtomicU32 = AtomicU32::new(1);
        let token = NEXT_MESSAGE_TOKEN.fetch_add(1, Ordering::Relaxed);

        Self {
            requested: AtomicBool::new(false),
            thread_id: Mutex::new(None),
            message_token: token.max(1) as usize,
        }
    }
}

struct Completion {
    completed: AtomicBool,
    sender: Sender<LoginOutcome>,
}

impl Completion {
    fn new(sender: Sender<LoginOutcome>) -> Self {
        Self {
            completed: AtomicBool::new(false),
            sender,
        }
    }

    fn try_complete(&self, outcome: LoginOutcome) -> bool {
        if self
            .completed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }

        let _ = self.sender.try_send(outcome);
        true
    }

    fn is_completed(&self) -> bool {
        self.completed.load(Ordering::Acquire)
    }

    fn try_complete_worker_error(&self, error: WorkerError) -> bool {
        let outcome = match error {
            WorkerError::Cancelled => LoginOutcome::Cancelled,
            WorkerError::Unavailable(message) => LoginOutcome::Failed(LoginFailure {
                kind: LoginFailureKind::WebViewUnavailable,
                message,
            }),
            WorkerError::Internal(message) => LoginOutcome::Failed(LoginFailure {
                kind: LoginFailureKind::Native,
                message,
            }),
        };
        self.try_complete(outcome)
    }
}

enum WorkerError {
    Cancelled,
    Unavailable(String),
    Internal(String),
}
