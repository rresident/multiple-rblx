use std::{
    env, fs, io,
    io::Write as _,
    path::{Path, PathBuf},
};

use time::OffsetDateTime;
use tracing_appender::{
    non_blocking::{NonBlocking, WorkerGuard},
    rolling::{RollingFileAppender, Rotation},
};
use tracing_subscriber::{
    EnvFilter,
    layer::SubscriberExt,
    util::{SubscriberInitExt, TryInitError},
};

const APPLICATION_DIRECTORY: &str = "MultipleRoblox";
const LOG_DIRECTORY: &str = "logs";
const LOG_FILE_PREFIX: &str = "multiple-roblox";
const LOG_FILE_SUFFIX: &str = "log";
const RETAINED_LOG_FILES: usize = 7;

#[must_use = "keep this value alive until the application exits"]
pub(crate) struct Diagnostics {
    log_path: Option<PathBuf>,
    _file_guard: Option<WorkerGuard>,
}

impl Diagnostics {
    pub(crate) fn log_path(&self) -> Option<&Path> {
        self.log_path.as_deref()
    }
}

pub(crate) fn init() -> Diagnostics {
    let (file_writer, file_guard, log_path, file_error) = match create_file_sink() {
        Ok((writer, guard, path)) => (Some(writer), Some(guard), Some(path), None),
        Err(error) => (None, None, None, Some(error)),
    };

    if let Err(error) = install_subscriber(file_writer) {
        write_console(format_args!(
            "Multiple Roblox diagnostics could not be initialized: {error}"
        ));
    }

    if let Some(path) = log_path.as_ref() {
        tracing::info!(
            target: "multiple_rblx::diagnostics",
            log_path = %path.display(),
            "diagnostics initialized"
        );
    } else if let Some(error) = file_error {
        tracing::warn!(
            target: "multiple_rblx::diagnostics",
            reason = %error,
            "file diagnostics unavailable; continuing with console diagnostics"
        );
        write_console(format_args!(
            "Multiple Roblox could not open its log file; console diagnostics remain active: \
             {error}"
        ));
    }

    Diagnostics {
        log_path,
        _file_guard: file_guard,
    }
}

fn install_subscriber(file_writer: Option<NonBlocking>) -> Result<(), TryInitError> {
    let console = tracing_subscriber::fmt::layer()
        .compact()
        .with_target(true)
        .with_thread_names(true);

    let file = file_writer.map(|writer| {
        tracing_subscriber::fmt::layer()
            .compact()
            .with_ansi(false)
            .with_target(true)
            .with_thread_names(true)
            .with_writer(writer)
    });

    tracing_subscriber::registry()
        .with(environment_filter())
        .with(console)
        .with(file)
        .try_init()
}

fn environment_filter() -> EnvFilter {
    let configured = env::var("RUST_LOG").ok();
    environment_filter_for(configured.as_deref())
}

fn environment_filter_for(configured: Option<&str>) -> EnvFilter {
    let default_level = if cfg!(debug_assertions) {
        "debug"
    } else {
        "info"
    };
    let application_level = configured
        .and_then(requested_application_level)
        .unwrap_or(default_level);

    EnvFilter::new(format!(
        "warn,multiple_rblx={application_level},ureq=off,ureq_proto=off"
    ))
}

fn requested_application_level(configured: &str) -> Option<&'static str> {
    let mut global_level = None;
    let mut application_level = None;

    for directive in configured.split(',').map(str::trim) {
        let (target, level) = directive
            .rsplit_once('=')
            .map_or(("", directive), |(target, level)| {
                (target.trim(), level.trim())
            });
        let Some(level) = normalized_level(level) else {
            continue;
        };

        if target.is_empty() {
            global_level = Some(level);
        } else if target == "multiple_rblx" || target.starts_with("multiple_rblx::") {
            application_level = Some(level);
        }
    }

    application_level.or(global_level)
}

fn normalized_level(level: &str) -> Option<&'static str> {
    match level.to_ascii_lowercase().as_str() {
        "off" => Some("off"),
        "error" => Some("error"),
        "warn" => Some("warn"),
        "info" => Some("info"),
        "debug" => Some("debug"),
        "trace" => Some("trace"),
        _ => None,
    }
}

fn create_file_sink() -> io::Result<(NonBlocking, WorkerGuard, PathBuf)> {
    let log_directory = local_log_directory()?;
    fs::create_dir_all(&log_directory)?;

    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix(LOG_FILE_PREFIX)
        .filename_suffix(LOG_FILE_SUFFIX)
        .max_log_files(RETAINED_LOG_FILES)
        .build(&log_directory)
        .map_err(io::Error::other)?;

    let log_path = current_utc_log_path(&log_directory);
    let (writer, guard) = tracing_appender::non_blocking(appender);
    Ok((writer, guard, log_path))
}

fn local_log_directory() -> io::Result<PathBuf> {
    let local_app_data = env::var_os("LOCALAPPDATA").ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "LOCALAPPDATA is unavailable for this process",
        )
    })?;

    Ok(PathBuf::from(local_app_data)
        .join(APPLICATION_DIRECTORY)
        .join(LOG_DIRECTORY))
}

fn current_utc_log_path(log_directory: &Path) -> PathBuf {
    let date = OffsetDateTime::now_utc().date();
    log_directory.join(format!("{LOG_FILE_PREFIX}.{date}.{LOG_FILE_SUFFIX}"))
}

fn write_console(arguments: std::fmt::Arguments<'_>) {
    let _ = writeln!(io::stderr().lock(), "{arguments}");
}

#[cfg(test)]
mod tests {
    use std::{
        io::Write,
        sync::{Arc, Mutex},
    };

    use tracing::Level;
    use tracing_subscriber::fmt::MakeWriter;

    use super::*;

    #[derive(Clone, Default)]
    struct CapturedOutput(Arc<Mutex<Vec<u8>>>);

    struct CapturedWriter(Arc<Mutex<Vec<u8>>>);

    impl<'writer> MakeWriter<'writer> for CapturedOutput {
        type Writer = CapturedWriter;

        fn make_writer(&'writer self) -> Self::Writer {
            CapturedWriter(self.0.clone())
        }
    }

    impl Write for CapturedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().expect("captured log lock").extend(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn wire_level_dependency_traces_cannot_be_enabled_by_rust_log() {
        let output = CapturedOutput::default();
        let subscriber = tracing_subscriber::registry()
            .with(environment_filter_for(Some(
                "trace,ureq_proto::client=trace,ureq=trace",
            )))
            .with(
                tracing_subscriber::fmt::layer()
                    .without_time()
                    .with_ansi(false)
                    .with_writer(output.clone()),
            );
        let canary = "ROBLOSECURITY-CANARY-MUST-NOT-BE-LOGGED";

        tracing::subscriber::with_default(subscriber, || {
            tracing::event!(
                target: "ureq_proto::client",
                Level::TRACE,
                request_bytes = canary
            );
            tracing::event!(
                target: "multiple_rblx::diagnostics::test",
                Level::TRACE,
                "application trace remains available"
            );
        });

        let bytes = output.0.lock().expect("captured log lock").clone();
        let rendered = String::from_utf8(bytes).expect("diagnostics should be UTF-8");
        assert!(!rendered.contains(canary));
        assert!(rendered.contains("application trace remains available"));
    }

    #[test]
    fn rust_log_only_controls_application_verbosity() {
        assert_eq!(
            requested_application_level("gpui=trace,multiple_rblx::linking=debug"),
            Some("debug")
        );
        assert_eq!(requested_application_level("trace"), Some("trace"));
        assert_eq!(requested_application_level("ureq_proto=trace"), None);
    }
}
