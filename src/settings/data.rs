use std::{env, fs, path::PathBuf};

use anyhow::{Context as _, Result};

const APPLICATION_DIRECTORY: &str = "MultipleRoblox";
const BROWSER_DATA: &str = "BrowserData";
const LOG_DIRECTORY: &str = "logs";
const PREFERENCES_FILE: &str = "preferences.json";

fn application_directory() -> Result<PathBuf> {
    let local_app_data =
        env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is unavailable for this process")?;
    Ok(PathBuf::from(local_app_data).join(APPLICATION_DIRECTORY))
}

pub(crate) fn clearable_bytes() -> u64 {
    let Ok(root) = application_directory() else {
        return 0;
    };
    directory_size(&root.join(BROWSER_DATA)) + directory_size(&root.join(LOG_DIRECTORY))
}

fn directory_size(path: &PathBuf) -> u64 {
    let Ok(entries) = fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.file_type() {
            Ok(kind) if kind.is_dir() => directory_size(&entry.path()),
            Ok(_) => entry.metadata().map(|data| data.len()).unwrap_or(0),
            Err(_) => 0,
        })
        .sum()
}

pub(crate) fn clear_cached_files() -> Result<u64> {
    let root = application_directory()?;
    let freed = clearable_bytes();

    remove_directory_contents(&root.join(BROWSER_DATA));
    remove_directory_contents(&root.join(LOG_DIRECTORY));

    tracing::info!(freed, "cleared cached files");
    Ok(freed)
}

fn remove_directory_contents(path: &PathBuf) {
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let target = entry.path();
        let removed = match entry.file_type() {
            Ok(kind) if kind.is_dir() => fs::remove_dir_all(&target),
            _ => fs::remove_file(&target),
        };
        if let Err(error) = removed {
            tracing::debug!(path = %target.display(), reason = %error, "file could not be removed");
        }
    }
}

pub(crate) fn stored_account_count(vault: &crate::security::SharedSessionVault) -> usize {
    vault.list().map(|accounts| accounts.len()).unwrap_or(0)
}

pub(crate) fn delete_everything(vault: &crate::security::SharedSessionVault) -> Result<()> {
    let accounts = vault.list().unwrap_or_default();
    let mut failures = 0;
    for account in &accounts {
        if let Err(error) = vault.delete(account.user_id) {
            failures += 1;
            tracing::error!(
                account_id = account.user_id,
                reason = %error,
                "stored sign-in could not be deleted"
            );
        }
    }
    tracing::info!(
        removed = accounts.len() - failures,
        failures,
        "deleted stored sign-ins"
    );

    if let Err(error) = super::set_start_with_windows(false, false) {
        tracing::warn!(reason = %error, "startup entry could not be removed");
    }

    let root = application_directory()?;
    let _ = fs::remove_file(root.join(PREFERENCES_FILE));
    remove_directory_contents(&root.join(BROWSER_DATA));
    remove_directory_contents(&root.join(LOG_DIRECTORY));

    if failures > 0 {
        anyhow::bail!("{failures} stored sign-in(s) could not be removed");
    }
    Ok(())
}

pub(crate) fn format_bytes(bytes: u64) -> String {
    const MEGABYTE: u64 = 1024 * 1024;
    const KILOBYTE: u64 = 1024;

    if bytes >= MEGABYTE {
        let whole = bytes / MEGABYTE;
        let tenth = (bytes % MEGABYTE) / (MEGABYTE / 10);
        return format!("{whole}.{tenth} MB");
    }
    if bytes >= KILOBYTE {
        return format!("{} KB", bytes / KILOBYTE);
    }
    format!("{bytes} bytes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_the_way_a_person_would_say_them() {
        assert_eq!(format_bytes(0), "0 bytes");
        assert_eq!(format_bytes(512), "512 bytes");
        assert_eq!(format_bytes(2048), "2 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(3 * 1024 * 1024 + 512 * 1024), "3.5 MB");
    }

    #[test]
    fn measuring_a_missing_directory_is_zero_not_an_error() {
        assert_eq!(
            directory_size(&PathBuf::from("Z:\\definitely\\not\\here")),
            0
        );
    }
}
