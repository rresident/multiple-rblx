use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

const LATEST_RELEASE_URL: &str =
    "https://api.github.com/repos/rresident/multiple-rblx/releases/latest";
const AGENT_NAME: &str = "multiple-rblx";
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_DOWNLOAD_BYTES: u64 = 128 * 1024 * 1024;
const MIN_DOWNLOAD_BYTES: usize = 1024;
const STAGED_STEM: &str = "multiple-rblx.update";
const RETIRED_STEM: &str = "multiple-rblx.outdated";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Release {
    pub(crate) version: String,
    pub(crate) download_url: String,
}

#[derive(Deserialize)]
struct LatestRelease {
    tag_name: String,
    #[serde(default)]
    assets: Vec<ReleaseAsset>,
}

#[derive(Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .into()
}

pub(crate) fn parse_version(raw: &str) -> Option<(u64, u64, u64)> {
    let trimmed = raw.trim().trim_start_matches(['v', 'V']);
    let mut parts = trimmed.split('.');
    let major = parts.next()?.trim().parse().ok()?;
    let minor = parts.next().unwrap_or("0").trim().parse().ok()?;
    let patch = parts
        .next()
        .unwrap_or("0")
        .split(['-', '+'])
        .next()?
        .trim()
        .parse()
        .ok()?;
    Some((major, minor, patch))
}

fn portable_asset(assets: Vec<ReleaseAsset>) -> Option<ReleaseAsset> {
    assets.into_iter().find(|asset| {
        let name = asset.name.to_ascii_lowercase();
        name.ends_with(".exe") && !name.contains("setup") && !name.contains("install")
    })
}

pub(crate) fn check(installed: &str) -> Result<Option<Release>> {
    let latest: LatestRelease = agent(CHECK_TIMEOUT)
        .get(LATEST_RELEASE_URL)
        .header("Accept", "application/vnd.github+json")
        .header("User-Agent", AGENT_NAME)
        .call()
        .context("GitHub could not be reached")?
        .body_mut()
        .read_json()
        .context("the release list could not be read")?;

    let Some(available) = parse_version(&latest.tag_name) else {
        bail!("the latest release has an unreadable version");
    };
    let Some(running) = parse_version(installed) else {
        bail!("this build has an unreadable version");
    };
    if available <= running {
        return Ok(None);
    }

    let Some(asset) = portable_asset(latest.assets) else {
        bail!("that release has no downloadable program");
    };

    Ok(Some(Release {
        version: latest.tag_name.trim_start_matches(['v', 'V']).to_owned(),
        download_url: asset.browser_download_url,
    }))
}

fn sibling(directory: &Path, stem: &str, extension: &str) -> PathBuf {
    directory.join(format!("{stem}.{extension}"))
}

pub(crate) fn install(download_url: &str) -> Result<PathBuf> {
    if !download_url.starts_with("https://") {
        bail!("refusing to download over an insecure connection");
    }

    let current = std::env::current_exe().context("locating this program")?;
    let directory = current
        .parent()
        .context("this program has no containing folder")?
        .to_path_buf();
    let extension = current
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("exe")
        .to_owned();

    let bytes = agent(DOWNLOAD_TIMEOUT)
        .get(download_url)
        .header("User-Agent", AGENT_NAME)
        .call()
        .context("the update could not be downloaded")?
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD_BYTES)
        .read_to_vec()
        .context("the update could not be read")?;

    if bytes.len() < MIN_DOWNLOAD_BYTES || !bytes.starts_with(b"MZ") {
        bail!("the download was not a Windows program");
    }

    let staged = sibling(&directory, STAGED_STEM, &extension);
    let retired = sibling(&directory, RETIRED_STEM, &extension);
    let _ = std::fs::remove_file(&retired);

    std::fs::write(&staged, &bytes).context("the update could not be saved")?;

    if let Err(error) = std::fs::rename(&current, &retired) {
        let _ = std::fs::remove_file(&staged);
        return Err(error).context("the running program could not be moved aside");
    }

    if let Err(error) = std::fs::rename(&staged, &current) {
        let _ = std::fs::rename(&retired, &current);
        let _ = std::fs::remove_file(&staged);
        return Err(error).context("the update could not be put in place");
    }

    Ok(current)
}

pub(crate) fn relaunch(executable: &Path) -> Result<()> {
    std::process::Command::new(executable)
        .spawn()
        .context("the updated program could not be started")?;
    Ok(())
}

pub(crate) fn discard_leftovers() {
    let Ok(current) = std::env::current_exe() else {
        return;
    };
    let Some(directory) = current.parent() else {
        return;
    };
    let extension = current
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("exe");
    let _ = std::fs::remove_file(sibling(directory, RETIRED_STEM, extension));
    let _ = std::fs::remove_file(sibling(directory, STAGED_STEM, extension));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_with_and_without_a_leading_v() {
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version(" v10.0.1 "), Some((10, 0, 1)));
        assert_eq!(parse_version("2.1"), Some((2, 1, 0)));
        assert_eq!(parse_version("3"), Some((3, 0, 0)));
        assert_eq!(parse_version("1.0.3-beta.1"), Some((1, 0, 3)));
        assert_eq!(parse_version("nightly"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn versions_order_numerically_not_lexically() {
        assert!(parse_version("1.0.10") > parse_version("1.0.9"));
        assert!(parse_version("1.10.0") > parse_version("1.9.9"));
        assert!(parse_version("2.0.0") > parse_version("1.99.99"));
        assert_eq!(parse_version("1.0.3"), parse_version("v1.0.3"));
    }

    fn asset(name: &str) -> ReleaseAsset {
        ReleaseAsset {
            name: name.into(),
            browser_download_url: format!("https://example.invalid/{name}"),
        }
    }

    #[test]
    fn the_installer_is_never_chosen_over_the_portable_program() {
        let picked = portable_asset(vec![
            asset("multiple-rblx-setup.exe"),
            asset("multiple-rblx-x64.exe"),
        ])
        .expect("a portable exe should be found");
        assert_eq!(picked.name, "multiple-rblx-x64.exe");

        assert!(portable_asset(vec![asset("multiple-rblx-setup.exe")]).is_none());
        assert!(portable_asset(vec![asset("notes.txt")]).is_none());
        assert!(portable_asset(Vec::new()).is_none());
    }

    #[test]
    #[ignore = "talks to the live GitHub API"]
    fn the_live_release_is_seen_as_newer_than_an_older_build() {
        let found = check("0.0.1")
            .expect("the release should be readable")
            .expect("an older build should see an update");
        assert!(
            parse_version(&found.version) >= parse_version("1.0.0"),
            "unexpected version {}",
            found.version
        );
        assert!(found.download_url.starts_with("https://"));
        assert!(found.download_url.ends_with(".exe"));

        assert_eq!(
            check("999.0.0").expect("the release should be readable"),
            None,
            "a newer build should not be offered an update"
        );
    }
}
