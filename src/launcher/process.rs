use std::{
    path::PathBuf,
    process::{Child, Command},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, bail};
use secrecy::{ExposeSecret as _, zeroize::Zeroizing};

use crate::accounts::service::PreparedLaunch;

const PLACE_LAUNCHER_URL: &str = "https://assetgame.roblox.com/game/PlaceLauncher.ashx";
const PLAYER_EXECUTABLE: &str = "RobloxPlayerBeta.exe";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LaunchTarget {
    pub(crate) place_id: u64,
}

pub(crate) fn launch_client(
    prepared: &PreparedLaunch,
    target: LaunchTarget,
) -> Result<TrackedClient> {
    if target.place_id == 0 {
        bail!("a place id is required to launch");
    }

    let player = roblox_player_path().context("locating the Roblox client")?;
    let tracker = browser_tracker_id();
    let before = running_client_pids();

    let uri = Zeroizing::new(build_launch_uri(
        prepared.ticket().expose_secret(),
        target.place_id,
        &tracker,
    ));

    let _child: Child = Command::new(&player)
        .arg(uri.as_str())
        .spawn()
        .with_context(|| format!("starting {}", player.display()))?;

    std::thread::sleep(ADOPT_SETTLE);

    for _ in 0..ADOPT_ATTEMPTS {
        if let Some(client) = adopt_new_client(&before) {
            tracing::debug!(pid = client.pid(), "adopted the Roblox client process");
            return Ok(client);
        }
        std::thread::sleep(ADOPT_INTERVAL);
    }

    bail!("Roblox started but no client process could be tracked")
}

const ADOPT_SETTLE: Duration = Duration::from_millis(3_500);
const ADOPT_ATTEMPTS: u32 = 40;
const ADOPT_INTERVAL: Duration = Duration::from_millis(250);

#[cfg(target_os = "windows")]
pub(crate) struct TrackedClient {
    pid: u32,
    handle: windows::Win32::Foundation::HANDLE,
}

#[cfg(target_os = "windows")]
unsafe impl Send for TrackedClient {}

#[cfg(target_os = "windows")]
impl TrackedClient {
    pub(crate) fn pid(&self) -> u32 {
        self.pid
    }

    pub(crate) fn has_exited(&self) -> bool {
        use windows::Win32::{Foundation::WAIT_OBJECT_0, System::Threading::WaitForSingleObject};
        unsafe { WaitForSingleObject(self.handle, 0) == WAIT_OBJECT_0 }
    }

    pub(crate) fn terminate(&self) {
        use windows::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};
        unsafe {
            if TerminateProcess(self.handle, 0).is_ok() {
                WaitForSingleObject(self.handle, TERMINATE_WAIT_MS);
            }
        }
    }
}

#[cfg(target_os = "windows")]
impl Drop for TrackedClient {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(target_os = "windows")]
impl std::fmt::Debug for TrackedClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TrackedClient")
            .field("pid", &self.pid)
            .finish()
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn running_client_pids() -> std::collections::HashSet<u32> {
    snapshot_pids(&[PLAYER_EXECUTABLE]).into_iter().collect()
}

#[cfg(target_os = "windows")]
pub(crate) fn client_is_playing() -> bool {
    !snapshot_pids(&[PLAYER_EXECUTABLE]).is_empty()
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn client_is_playing() -> bool {
    false
}

#[cfg(target_os = "windows")]
fn adopt_new_client(before: &std::collections::HashSet<u32>) -> Option<TrackedClient> {
    use windows::Win32::{
        Foundation::WAIT_OBJECT_0,
        System::Threading::{
            OpenProcess, PROCESS_ACCESS_RIGHTS, PROCESS_TERMINATE, WaitForSingleObject,
        },
    };
    const SYNCHRONIZE: PROCESS_ACCESS_RIGHTS = PROCESS_ACCESS_RIGHTS(0x0010_0000);

    let mut candidates = snapshot_pids(&[PLAYER_EXECUTABLE])
        .into_iter()
        .filter(|pid| !before.contains(pid))
        .collect::<Vec<_>>();
    candidates.sort_unstable_by(|left, right| right.cmp(left));

    for pid in candidates {
        let Ok(handle) = (unsafe { OpenProcess(PROCESS_TERMINATE | SYNCHRONIZE, false, pid) })
        else {
            continue;
        };

        if unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0 {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(handle);
            }
            continue;
        }

        return Some(TrackedClient { pid, handle });
    }
    None
}

#[cfg(target_os = "windows")]
fn snapshot_pids(names: &[&str]) -> Vec<u32> {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
            TH32CS_SNAPPROCESS,
        },
    };

    let Ok(snapshot) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
        return Vec::new();
    };

    let mut entry = PROCESSENTRY32W {
        dwSize: u32::try_from(size_of::<PROCESSENTRY32W>()).unwrap_or(0),
        ..Default::default()
    };

    let mut pids = Vec::new();
    if unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok() {
        loop {
            let length = entry
                .szExeFile
                .iter()
                .position(|character| *character == 0)
                .unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..length]);
            if names
                .iter()
                .any(|candidate| candidate.eq_ignore_ascii_case(&name))
            {
                pids.push(entry.th32ProcessID);
            }

            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }

    unsafe {
        let _ = CloseHandle(snapshot);
    }
    pids
}

#[cfg(target_os = "windows")]
const TERMINATE_WAIT_MS: u32 = 3_000;

#[cfg(target_os = "windows")]
const CLIENT_PROCESS_NAMES: [&str; 2] = ["RobloxPlayerBeta.exe", "RobloxCrashHandler.exe"];

#[cfg(target_os = "windows")]
pub(crate) fn close_running_clients() -> Result<usize> {
    use windows::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            OpenProcess, PROCESS_ACCESS_RIGHTS, PROCESS_TERMINATE, TerminateProcess,
            WaitForSingleObject,
        },
    };
    const SYNCHRONIZE: PROCESS_ACCESS_RIGHTS = PROCESS_ACCESS_RIGHTS(0x0010_0000);

    let mut closed = 0;
    for pid in snapshot_pids(&CLIENT_PROCESS_NAMES) {
        let Ok(handle) = (unsafe { OpenProcess(PROCESS_TERMINATE | SYNCHRONIZE, false, pid) })
        else {
            tracing::warn!(pid, "Roblox process could not be opened to close it");
            continue;
        };
        let terminated = unsafe { TerminateProcess(handle, 0) }.is_ok();
        if terminated {
            unsafe {
                WaitForSingleObject(handle, TERMINATE_WAIT_MS);
            }
            closed += 1;
        }
        unsafe {
            let _ = CloseHandle(handle);
        }
    }

    tracing::info!(closed, "closed running Roblox client processes");
    Ok(closed)
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn close_running_clients() -> Result<usize> {
    bail!("closing Roblox is only supported on Windows")
}

pub(crate) fn roblox_player_path() -> Result<PathBuf> {
    if let Ok(registered) = registered_player_command()
        && let Some(directory) = registered.parent()
    {
        let player = directory.join(PLAYER_EXECUTABLE);
        if player.is_file() {
            return Ok(player);
        }
    }

    newest_installed_player()
        .context("could not find RobloxPlayerBeta.exe; install or repair Roblox and try again")
}

fn newest_installed_player() -> Result<PathBuf> {
    let local_app_data = std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is unavailable")?;
    let versions = PathBuf::from(local_app_data)
        .join("Roblox")
        .join("Versions");

    let mut best: Option<(SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(&versions)
        .with_context(|| format!("reading {}", versions.display()))?
        .flatten()
    {
        let player = entry.path().join(PLAYER_EXECUTABLE);
        if !player.is_file() {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH);
        if best.as_ref().is_none_or(|(best, _)| modified > *best) {
            best = Some((modified, player));
        }
    }

    match best {
        Some((_, player)) => Ok(player),
        None => bail!("no {PLAYER_EXECUTABLE} under {}", versions.display()),
    }
}

#[cfg(target_os = "windows")]
fn registered_player_command() -> Result<PathBuf> {
    use windows::{
        Win32::System::Registry::{HKEY_CLASSES_ROOT, RRF_RT_REG_SZ, RegGetValueW},
        core::w,
    };

    let mut size: u32 = 0;
    let status = unsafe {
        RegGetValueW(
            HKEY_CLASSES_ROOT,
            w!("roblox\\DefaultIcon"),
            None,
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    };
    if status.is_err() || size == 0 {
        bail!("Roblox is not registered as a protocol handler on this machine");
    }

    let mut buffer = vec![0_u16; (size as usize).div_ceil(2)];
    let mut written = size;
    let status = unsafe {
        RegGetValueW(
            HKEY_CLASSES_ROOT,
            w!("roblox\\DefaultIcon"),
            None,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut written),
        )
    };
    if status.is_err() {
        bail!("the Roblox protocol registration could not be read");
    }

    let characters = (written as usize / 2).min(buffer.len());
    let raw = String::from_utf16_lossy(&buffer[..characters]);
    let trimmed = raw.trim_end_matches('\0').trim().trim_matches('"');
    if trimmed.is_empty() {
        bail!("the Roblox protocol registration is empty");
    }

    Ok(PathBuf::from(trimmed))
}

#[cfg(not(target_os = "windows"))]
fn registered_player_command() -> Result<PathBuf> {
    bail!("launching Roblox is only supported on Windows")
}

fn build_launch_uri(ticket: &str, place_id: u64, tracker: &str) -> String {
    let place_launcher = format!(
        "{PLACE_LAUNCHER_URL}?request=RequestGame&browserTrackerId={tracker}\
         &placeId={place_id}&isPlayTogetherGame=false"
    );

    format!(
        "roblox-player:1+launchmode:play+gameinfo:{ticket}+launchtime:{}\
         +placelauncherurl:{}+browsertrackerid:{tracker}\
         +robloxLocale:en_us+gameLocale:en_us+channel:+LaunchExp:InApp",
        launch_time_millis(),
        percent_encode(&place_launcher),
    )
}

fn launch_time_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis()
}

fn browser_tracker_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .subsec_nanos() as u64;
    let mixed = nanos
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(u64::from(std::process::id()));
    format!("{}", 100_000_000_000 + (mixed % 799_999_999_999))
}

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len() * 3);
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_encoding_escapes_every_uri_delimiter() {
        assert_eq!(percent_encode("abc-._~"), "abc-._~");
        assert_eq!(percent_encode("a+b"), "a%2Bb");
        assert_eq!(percent_encode("a b"), "a%20b");
        assert_eq!(percent_encode("a:b"), "a%3Ab");
        assert_eq!(
            percent_encode("https://x/y?a=1&b=2"),
            "https%3A%2F%2Fx%2Fy%3Fa%3D1%26b%3D2"
        );
    }

    #[test]
    fn launch_uri_has_every_required_field_and_no_stray_separators() {
        let uri = build_launch_uri("TICKET123", 4924922222, "123456789012");

        assert!(uri.starts_with("roblox-player:1+launchmode:play+"));
        for field in [
            "gameinfo:TICKET123",
            "browsertrackerid:123456789012",
            "robloxLocale:en_us",
            "gameLocale:en_us",
            "channel:",
            "LaunchExp:InApp",
        ] {
            assert!(uri.contains(field), "missing {field} in {uri}");
        }

        let encoded = uri
            .split("+placelauncherurl:")
            .nth(1)
            .and_then(|rest| rest.split('+').next())
            .expect("placelauncherurl field should be present");
        assert!(!encoded.contains("://"), "url should be encoded");
        assert!(encoded.contains("%3A%2F%2F"));
        assert!(encoded.contains(&format!("placeId%3D{}", 4924922222_u64)));
    }

    #[test]
    fn browser_tracker_ids_are_twelve_digits() {
        let tracker = browser_tracker_id();
        assert_eq!(tracker.len(), 12, "{tracker}");
        assert!(tracker.bytes().all(|byte| byte.is_ascii_digit()));
    }

    #[test]
    #[ignore = "requires Roblox to be installed"]
    fn the_installed_client_can_be_located() {
        let player = roblox_player_path().expect("Roblox client should be locatable");
        println!("found client at {}", player.display());
        assert!(player.is_file());
        assert!(player.ends_with(PLAYER_EXECUTABLE));
    }
}
