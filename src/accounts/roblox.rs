use std::{collections::HashMap, time::Duration};

use anyhow::{Context as _, Result};
use secrecy::{ExposeSecret as _, SecretString, zeroize::Zeroizing};
use serde::{Deserialize, Serialize};

use crate::security::MAX_SESSION_BYTES;

use super::{
    model::FetchedProfile,
    service::{RobloxGateway, VerificationIssue, VerifiedRobloxAccount, VerifySessionFailure},
};

const AUTHENTICATED_USER_URL: &str = "https://users.roblox.com/v1/users/authenticated";
const USERS_URL: &str = "https://users.roblox.com/v1/users";
const AUTHENTICATION_TICKET_URL: &str = "https://auth.roblox.com/v1/authentication-ticket/";
const TICKET_REFERER: &str = "https://www.roblox.com/games/4924922222/Brookhaven-RP";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const MAX_HEADSHOT_BYTES: u64 = 2 * 1024 * 1024;
const MAX_TICKET_BYTES: usize = 4_096;
const TICKET_ATTEMPTS: u32 = 3;
const TICKET_RETRY_DELAY: Duration = Duration::from_millis(400);

pub(super) struct RobloxClient;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UsersRequest {
    user_ids: Vec<u64>,
    exclude_banned_users: bool,
}

#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    data: Vec<T>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthenticatedRobloxUser {
    id: u64,
    name: String,
}

#[derive(Debug, Deserialize)]
struct RobloxUser {
    id: u64,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RobloxHeadshot {
    target_id: u64,
    state: String,
    image_url: Option<String>,
}

impl RobloxGateway for RobloxClient {
    fn verify_session(
        &self,
        session: &SecretString,
    ) -> Result<VerifiedRobloxAccount, VerifySessionFailure> {
        if !is_header_safe_session(session.expose_secret()) {
            return Err(VerifySessionFailure::Rejected);
        }
        let agent = verification_agent();
        let cookie_header = Zeroizing::new(format!(".ROBLOSECURITY={}", session.expose_secret()));
        let response = agent
            .get(AUTHENTICATED_USER_URL)
            .header("Accept", "application/json")
            .header("Cookie", cookie_header.as_str())
            .call();

        let mut response = match response {
            Ok(response) if response.status().as_u16() == 200 => response,
            Err(ureq::Error::StatusCode(401 | 403)) => {
                return Err(VerifySessionFailure::Rejected);
            }
            Ok(response) => {
                return Err(VerifySessionFailure::Unavailable(
                    VerificationIssue::HttpStatus(response.status().as_u16()),
                ));
            }
            Err(error) => {
                return Err(VerifySessionFailure::Unavailable(verification_issue(
                    &error,
                )));
            }
        };

        let user: AuthenticatedRobloxUser = response
            .body_mut()
            .read_json()
            .map_err(|_| VerifySessionFailure::Unavailable(VerificationIssue::InvalidResponse))?;
        if user.id == 0 || user.name.trim().is_empty() {
            return Err(VerifySessionFailure::Rejected);
        }

        Ok(VerifiedRobloxAccount {
            user_id: user.id,
            username: user.name,
        })
    }

    fn fetch_profiles(&self, user_ids: &[u64]) -> Result<Vec<FetchedProfile>> {
        fetch_profiles(user_ids)
    }

    fn create_authentication_ticket(
        &self,
        session: &SecretString,
    ) -> Result<SecretString, VerifySessionFailure> {
        create_authentication_ticket(session)
    }
}

fn create_authentication_ticket(
    session: &SecretString,
) -> Result<SecretString, VerifySessionFailure> {
    if !is_header_safe_session(session.expose_secret()) {
        return Err(VerifySessionFailure::Rejected);
    }

    let agent = ticket_agent();
    let cookie_header = Zeroizing::new(format!(".ROBLOSECURITY={}", session.expose_secret()));
    let mut csrf: Option<Zeroizing<String>> = None;
    let mut last_status = 0_u16;

    for attempt in 0..TICKET_ATTEMPTS {
        let mut request = agent
            .post(AUTHENTICATION_TICKET_URL)
            .header("Referer", TICKET_REFERER)
            .header("Cookie", cookie_header.as_str())
            .header("Content-Type", "application/json");
        if let Some(token) = csrf.as_ref() {
            request = request
                .header("X-CSRF-TOKEN", token.as_str())
                .header("RBXAuthenticationNegotiation", "1");
        }

        let response = request
            .send("{}")
            .map_err(|error| VerifySessionFailure::Unavailable(verification_issue(&error)))?;
        last_status = response.status().as_u16();

        if let Some(ticket) = response
            .headers()
            .get("rbx-authentication-ticket")
            .and_then(|value| value.to_str().ok())
        {
            return if is_launch_safe_ticket(ticket) {
                Ok(SecretString::from(ticket.to_owned()))
            } else {
                tracing::warn!(
                    length = ticket.len(),
                    "Roblox returned a ticket that cannot be placed in a launch URI"
                );
                Err(VerifySessionFailure::Unavailable(
                    VerificationIssue::InvalidResponse,
                ))
            };
        }

        if let Some(fresh) = response
            .headers()
            .get("x-csrf-token")
            .and_then(|value| value.to_str().ok())
        {
            csrf = Some(Zeroizing::new(fresh.to_owned()));
            continue;
        }

        if last_status == 401 || last_status == 403 {
            return Err(VerifySessionFailure::Rejected);
        }
        if !is_retryable_status(last_status) {
            break;
        }
        tracing::warn!(
            status = last_status,
            attempt = attempt + 1,
            "launch ticket request was throttled or failed; retrying"
        );
        std::thread::sleep(TICKET_RETRY_DELAY);
    }

    Err(VerifySessionFailure::Unavailable(
        VerificationIssue::HttpStatus(last_status),
    ))
}

fn is_retryable_status(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

fn is_launch_safe_ticket(ticket: &str) -> bool {
    !ticket.is_empty()
        && ticket.len() <= MAX_TICKET_BYTES
        && ticket.is_ascii()
        && !ticket.bytes().any(|byte| {
            byte.is_ascii_control() || byte.is_ascii_whitespace() || matches!(byte, b'+' | b':')
        })
}

fn verification_issue(error: &ureq::Error) -> VerificationIssue {
    match error {
        ureq::Error::Timeout(_) => VerificationIssue::Timeout,
        ureq::Error::HostNotFound => VerificationIssue::Dns,
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) | ureq::Error::TlsRequired => {
            VerificationIssue::Tls
        }
        ureq::Error::StatusCode(status) => VerificationIssue::HttpStatus(*status),
        ureq::Error::Io(_) => VerificationIssue::Io,
        ureq::Error::Protocol(_) => VerificationIssue::Protocol,
        ureq::Error::Json(_) => VerificationIssue::InvalidResponse,
        _ => VerificationIssue::Protocol,
    }
}

fn is_header_safe_session(session: &str) -> bool {
    !session.is_empty()
        && session.len() <= MAX_SESSION_BYTES
        && !session
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b';')
}

fn verification_agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .max_redirects(0)
        .build();
    config.into()
}

fn ticket_agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .max_redirects(0)
        .http_status_as_error(false)
        .build();
    config.into()
}

fn public_agent() -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build();
    config.into()
}

fn fetch_profiles(user_ids: &[u64]) -> Result<Vec<FetchedProfile>> {
    if user_ids.is_empty() {
        return Ok(Vec::new());
    }

    let agent = public_agent();
    let request = UsersRequest {
        user_ids: user_ids.to_vec(),
        exclude_banned_users: false,
    };
    let mut users_response = agent
        .post(USERS_URL)
        .header("Accept", "application/json")
        .send_json(&request)
        .context("fetching Roblox users")?;
    let users: ApiResponse<RobloxUser> = users_response
        .body_mut()
        .read_json()
        .context("reading Roblox user response")?;
    let headshots = fetch_headshots(&agent, user_ids).unwrap_or_default();

    Ok(join_profiles(users.data, headshots)
        .into_iter()
        .map(|profile| {
            let avatar_png = profile
                .avatar_url
                .as_deref()
                .and_then(|url| download_headshot(&agent, url));
            FetchedProfile {
                id: profile.id,
                username: profile.username,
                avatar_png,
            }
        })
        .collect())
}

fn fetch_headshots(agent: &ureq::Agent, user_ids: &[u64]) -> Result<Vec<RobloxHeadshot>> {
    let id_list = user_ids
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let url = format!(
        "https://thumbnails.roblox.com/v1/users/avatar-headshot?userIds={id_list}&size=150x150&format=Png&isCircular=false&includeBackground=false"
    );
    let mut response = agent
        .get(&url)
        .header("Accept", "application/json")
        .call()
        .context("fetching Roblox headshots")?;
    let response: ApiResponse<RobloxHeadshot> = response
        .body_mut()
        .read_json()
        .context("reading Roblox headshot response")?;
    Ok(response.data)
}

#[derive(Debug, PartialEq, Eq)]
struct ResolvedProfile {
    id: u64,
    username: String,
    avatar_url: Option<String>,
}

fn join_profiles(users: Vec<RobloxUser>, headshots: Vec<RobloxHeadshot>) -> Vec<ResolvedProfile> {
    let mut avatar_urls = headshots
        .into_iter()
        .filter(|headshot| headshot.state == "Completed")
        .filter_map(|headshot| headshot.image_url.map(|url| (headshot.target_id, url)))
        .collect::<HashMap<_, _>>();

    users
        .into_iter()
        .map(|user| ResolvedProfile {
            id: user.id,
            username: user.name,
            avatar_url: avatar_urls.remove(&user.id),
        })
        .collect()
}

fn download_headshot(agent: &ureq::Agent, url: &str) -> Option<Vec<u8>> {
    if !url.starts_with("https://") {
        return None;
    }

    agent
        .get(url)
        .header("Accept", "image/png")
        .call()
        .ok()?
        .body_mut()
        .with_config()
        .limit(MAX_HEADSHOT_BYTES)
        .read_to_vec()
        .ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn launch_tickets_accept_base64url_and_reject_only_uri_breakers() {
        let realistic = "aB9-_".repeat(85);
        assert!(is_launch_safe_ticket(&realistic));

        assert!(!is_launch_safe_ticket(""));
        assert!(!is_launch_safe_ticket("has+plus"), "+ separates URI fields");
        assert!(
            !is_launch_safe_ticket("has:colon"),
            ": separates key from value"
        );
        assert!(!is_launch_safe_ticket("has space"));
        assert!(!is_launch_safe_ticket("has\nnewline"));
        assert!(!is_launch_safe_ticket(&"a".repeat(MAX_TICKET_BYTES + 1)));

        assert!(is_launch_safe_ticket("has%percent"));
        assert!(is_launch_safe_ticket("has&ampersand"));
        assert!(is_launch_safe_ticket("has=equals"));
    }

    #[test]
    fn only_throttling_and_server_faults_are_retried() {
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(503));
        assert!(!is_retryable_status(200));
        assert!(!is_retryable_status(400));
        assert!(!is_retryable_status(403));
        assert!(!is_retryable_status(404));
    }

    #[test]
    #[ignore = "hits Roblox with every stored session"]
    fn probe_ticket_flow_for_every_stored_account() {
        use crate::security::system_vault;

        let vault = system_vault();
        let accounts = vault.list().expect("vault should list");
        println!("{} stored account(s)", accounts.len());

        for account in accounts {
            let user_id = account.user_id;
            println!("\n=== account {user_id} ===");
            let Ok(Some(session)) = vault.load_session(user_id) else {
                println!("  no stored session");
                continue;
            };

            let agent = ticket_agent();
            let cookie = Zeroizing::new(format!(".ROBLOSECURITY={}", session.expose_secret()));

            let first = agent
                .post(AUTHENTICATION_TICKET_URL)
                .header("Referer", TICKET_REFERER)
                .header("Cookie", cookie.as_str())
                .header("Content-Type", "application/json")
                .send("{}");

            let Ok(first) = first else {
                println!("  [1] transport error");
                continue;
            };
            let csrf = first
                .headers()
                .get("x-csrf-token")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            println!(
                "  [1] status={} csrf={:?} ticket_present={} ratelimit_remaining={:?}",
                first.status().as_u16(),
                csrf.as_ref().map(String::len),
                first.headers().contains_key("rbx-authentication-ticket"),
                first
                    .headers()
                    .get("x-ratelimit-remaining")
                    .and_then(|value| value.to_str().ok())
            );

            let Some(csrf) = csrf else {
                println!("  [1] no csrf token, cannot continue");
                continue;
            };

            let second = agent
                .post(AUTHENTICATION_TICKET_URL)
                .header("Referer", TICKET_REFERER)
                .header("Cookie", cookie.as_str())
                .header("X-CSRF-TOKEN", csrf.as_str())
                .header("RBXAuthenticationNegotiation", "1")
                .header("Content-Type", "application/json")
                .send("{}");

            let Ok(second) = second else {
                println!("  [2] transport error");
                continue;
            };
            let status = second.status().as_u16();
            let ticket = second
                .headers()
                .get("rbx-authentication-ticket")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);

            match ticket {
                Some(ticket) => {
                    let mut punctuation = ticket
                        .chars()
                        .filter(|character| !character.is_ascii_alphanumeric())
                        .collect::<Vec<_>>();
                    punctuation.sort_unstable();
                    punctuation.dedup();
                    println!(
                        "  [2] status={status} ticket_len={} punctuation={:?}",
                        ticket.len(),
                        punctuation
                    );
                    println!(
                        "  [2] passes is_launch_safe_ticket = {}",
                        is_launch_safe_ticket(&ticket)
                    );
                }
                None => {
                    println!("  [2] status={status} NO TICKET HEADER");
                    let names = second
                        .headers()
                        .keys()
                        .map(|name| name.to_string())
                        .collect::<Vec<_>>();
                    println!("  [2] headers={names:?}");
                }
            }
        }
    }

    fn user(id: u64, name: &str) -> RobloxUser {
        RobloxUser {
            id,
            name: name.to_string(),
        }
    }

    fn headshot(id: u64, state: &str, url: Option<&str>) -> RobloxHeadshot {
        RobloxHeadshot {
            target_id: id,
            state: state.to_string(),
            image_url: url.map(str::to_string),
        }
    }

    #[test]
    fn profile_join_uses_usernames_and_matches_headshots_by_id() {
        let profiles = join_profiles(
            vec![user(99, "ninety-nine"), user(7, "seven")],
            vec![
                headshot(7, "Completed", Some("https://cdn/7.png")),
                headshot(99, "Pending", Some("https://cdn/99.png")),
            ],
        );

        assert_eq!(
            profiles,
            vec![
                ResolvedProfile {
                    id: 99,
                    username: "ninety-nine".into(),
                    avatar_url: None,
                },
                ResolvedProfile {
                    id: 7,
                    username: "seven".into(),
                    avatar_url: Some("https://cdn/7.png".into()),
                },
            ]
        );
    }

    #[test]
    fn authenticated_response_uses_username_not_display_name() {
        let user: AuthenticatedRobloxUser = serde_json::from_value(json!({
            "id": 42,
            "name": "account_name",
            "displayName": "Different"
        }))
        .expect("Roblox user fixture should deserialize");

        assert_eq!(user.name, "account_name");
    }

    #[test]
    fn users_request_uses_roblox_field_names() {
        let request = UsersRequest {
            user_ids: vec![42, 7],
            exclude_banned_users: false,
        };
        assert_eq!(
            serde_json::to_value(request).expect("request should serialize"),
            json!({
                "userIds": [42, 7],
                "excludeBannedUsers": false
            })
        );
    }

    #[test]
    fn headshots_require_completed_https_results() {
        let profiles = join_profiles(
            vec![user(42, "account")],
            vec![headshot(
                42,
                "Completed",
                Some("http://insecure.example/avatar.png"),
            )],
        );

        assert_eq!(
            profiles[0].avatar_url.as_deref(),
            Some("http://insecure.example/avatar.png")
        );
        assert!(download_headshot(&public_agent(), "http://example.test").is_none());
    }

    #[test]
    fn no_profile_request_is_made_for_an_empty_account_list() {
        assert!(
            fetch_profiles(&[])
                .expect("empty request should succeed")
                .is_empty()
        );
    }

    #[test]
    fn session_header_value_rejects_delimiters_controls_and_oversize_values() {
        assert!(is_header_safe_session("valid-session-value"));
        assert!(!is_header_safe_session(""));
        assert!(!is_header_safe_session("value;another-cookie=bad"));
        assert!(!is_header_safe_session("value\r\nInjected: bad"));
        assert!(!is_header_safe_session(&"x".repeat(MAX_SESSION_BYTES + 1)));
    }
}
