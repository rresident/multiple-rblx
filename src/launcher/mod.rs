mod process;
mod singleton;

pub(crate) use process::{
    LaunchTarget, TrackedClient, client_is_playing, close_running_clients, launch_client,
};
pub(crate) use singleton::{ArmOutcome, MultiInstanceGuard, arm};
