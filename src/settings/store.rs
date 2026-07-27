use std::{
    env, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{Context as _, Result};

use super::{Preferences, SCHEMA_VERSION, SettingsStore};

const APPLICATION_DIRECTORY: &str = "MultipleRoblox";
const PREFERENCES_FILE: &str = "preferences.json";
const TEMPORARY_FILE: &str = "preferences.json.tmp";

const MAX_PREFERENCES_BYTES: u64 = 512 * 1024;

pub(crate) type SharedSettings = Arc<dyn SettingsStore>;

pub(crate) fn system_settings() -> SharedSettings {
    Arc::new(FileSettingsStore)
}

struct FileSettingsStore;

impl SettingsStore for FileSettingsStore {
    fn load(&self) -> Result<Preferences> {
        let path = preferences_path()?;

        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(Preferences::default());
            }
            Err(error) => return Err(error).context("reading preferences metadata"),
        };

        if metadata.len() > MAX_PREFERENCES_BYTES {
            anyhow::bail!("preferences file is implausibly large; ignoring it");
        }

        let contents = fs::read_to_string(&path).context("reading preferences")?;
        let mut preferences: Preferences =
            serde_json::from_str(&contents).context("parsing preferences")?;
        preferences.version = SCHEMA_VERSION;
        Ok(preferences)
    }

    fn save(&self, preferences: &Preferences) -> Result<()> {
        let path = preferences_path()?;
        let directory = path
            .parent()
            .context("preferences path has no parent directory")?;
        fs::create_dir_all(directory).context("creating the application data directory")?;

        let mut document = preferences.clone();
        document.version = SCHEMA_VERSION;
        let encoded = serde_json::to_string_pretty(&document).context("encoding preferences")?;

        let temporary = directory.join(TEMPORARY_FILE);
        fs::write(&temporary, encoded).context("writing preferences")?;
        if let Err(error) = fs::rename(&temporary, &path) {
            let _ = fs::remove_file(&temporary);
            return Err(error).context("replacing preferences");
        }

        Ok(())
    }
}

fn preferences_path() -> Result<PathBuf> {
    Ok(application_directory()?.join(PREFERENCES_FILE))
}

fn application_directory() -> Result<PathBuf> {
    let local_app_data =
        env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is unavailable for this process")?;
    Ok(Path::new(&local_app_data).join(APPLICATION_DIRECTORY))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SavedGame;

    #[test]
    fn a_missing_preferences_file_loads_defaults_rather_than_failing() {
        let unique = format!("mrblx-settings-test-{}", std::process::id());
        let root = env::temp_dir().join(unique);
        fs::create_dir_all(&root).expect("test directory should be creatable");

        let previous = env::var_os("LOCALAPPDATA");
        unsafe { env::set_var("LOCALAPPDATA", &root) };

        let store = FileSettingsStore;
        let loaded = store.load().expect("missing file should load defaults");
        assert_eq!(loaded, Preferences::default());

        let mut updated = Preferences {
            multiple_instances_enabled: true,
            ..Preferences::default()
        };
        updated.record_launch(&SavedGame {
            universe_id: 5,
            root_place_id: 50,
            name: "Test".into(),
            icon_url: None,
        });
        store.save(&updated).expect("save should succeed");

        let reloaded = store.load().expect("reload should succeed");
        assert!(reloaded.multiple_instances_enabled);
        assert_eq!(reloaded.recents.len(), 1);
        assert_eq!(reloaded.version, SCHEMA_VERSION);

        assert!(
            !root
                .join(APPLICATION_DIRECTORY)
                .join(TEMPORARY_FILE)
                .exists()
        );

        match previous {
            Some(value) => unsafe { env::set_var("LOCALAPPDATA", value) },
            None => unsafe { env::remove_var("LOCALAPPDATA") },
        }
        let _ = fs::remove_dir_all(&root);
    }
}
