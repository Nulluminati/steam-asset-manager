use std::collections::HashSet;

use serde::Deserialize;

use super::library::{EntrySource, LibraryEntry, sort_entries};

pub const STEAM_API_BASE: &str = "https://api.steampowered.com";

#[derive(Debug, thiserror::Error)]
pub enum OwnedError {
    #[error("Steam rejected the Steam Web API key (HTTP {0})")]
    BadKey(u16),
    #[error("the Steam profile hides its game details; make them public to list owned games")]
    PrivateProfile,
    #[error("the Steam Web API returned HTTP {0}")]
    Status(u16),
    #[error("the Steam Web API request failed: {0}")]
    Request(reqwest::Error),
}

/// Calls `GetOwnedGames` and returns the owned games with
/// `installed: false`. `base_url` is [`STEAM_API_BASE`] outside tests.
pub async fn fetch_owned(
    key: &str,
    steam_id64: u64,
    base_url: &str,
) -> Result<Vec<LibraryEntry>, OwnedError> {
    let url = format!(
        "{}/IPlayerService/GetOwnedGames/v1/",
        base_url.trim_end_matches('/')
    );
    // The URL holds the key, so every error drops the URL before it is shown.
    let response = reqwest::Client::new()
        .get(url)
        .query(&[
            ("key", key.to_string()),
            ("steamid", steam_id64.to_string()),
            ("include_appinfo", "1".to_string()),
            ("include_played_free_games", "1".to_string()),
        ])
        .send()
        .await
        .map_err(request_error)?;

    let status = response.status().as_u16();
    match status {
        200..=299 => {}
        401 | 403 => return Err(OwnedError::BadKey(status)),
        _ => return Err(OwnedError::Status(status)),
    }

    let body: OwnedGamesBody = response.json().await.map_err(request_error)?;
    match (body.response.games, body.response.game_count) {
        (Some(games), _) => Ok(games
            .into_iter()
            .map(|game| LibraryEntry {
                appid: game.appid,
                name: game.name,
                source: EntrySource::Steam,
                installed: false,
            })
            .collect()),
        // A public profile with no games sends a count and no list.
        (None, Some(_)) => Ok(Vec::new()),
        // A private profile sends an empty `response` object.
        (None, None) => Err(OwnedError::PrivateProfile),
    }
}

fn request_error(err: reqwest::Error) -> OwnedError {
    OwnedError::Request(err.without_url())
}

#[derive(Deserialize)]
struct OwnedGamesBody {
    response: OwnedGamesResponse,
}

#[derive(Deserialize)]
struct OwnedGamesResponse {
    game_count: Option<u32>,
    games: Option<Vec<OwnedGame>>,
}

#[derive(Deserialize)]
struct OwnedGame {
    appid: u32,
    name: String,
}

/// Adds the owned games that are not installed to the installed list.
/// Installed entries stay as they are. The result is sorted by name.
pub fn merge(installed: Vec<LibraryEntry>, owned: Vec<LibraryEntry>) -> Vec<LibraryEntry> {
    let installed_ids: HashSet<u32> = installed.iter().map(|entry| entry.appid).collect();
    let mut merged = installed;
    merged.extend(
        owned
            .into_iter()
            .filter(|entry| !installed_ids.contains(&entry.appid))
            .map(|entry| LibraryEntry {
                installed: false,
                ..entry
            }),
    );
    sort_entries(&mut merged);
    merged
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const STEAM_ID: u64 = 76_561_197_960_287_930;

    fn entry(appid: u32, name: &str, installed: bool) -> LibraryEntry {
        LibraryEntry {
            appid,
            name: name.to_string(),
            source: EntrySource::Steam,
            installed,
        }
    }

    async fn server_with(response: ResponseTemplate) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/IPlayerService/GetOwnedGames/v1/"))
            .and(query_param("key", "test-key"))
            .and(query_param("steamid", STEAM_ID.to_string()))
            .and(query_param("include_appinfo", "1"))
            .and(query_param("include_played_free_games", "1"))
            .respond_with(response)
            .expect(1)
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn normal_list() {
        let body = serde_json::json!({
            "response": {
                "game_count": 2,
                "games": [
                    {"appid": 620, "name": "Portal 2", "playtime_forever": 10},
                    {"appid": 440, "name": "Team Fortress 2", "playtime_forever": 0}
                ]
            }
        });
        let server = server_with(ResponseTemplate::new(200).set_body_json(body)).await;
        let games = fetch_owned("test-key", STEAM_ID, &server.uri())
            .await
            .unwrap();
        assert_eq!(
            games,
            vec![
                entry(620, "Portal 2", false),
                entry(440, "Team Fortress 2", false),
            ]
        );
    }

    #[tokio::test]
    async fn zero_games_is_an_empty_list() {
        let body = serde_json::json!({"response": {"game_count": 0}});
        let server = server_with(ResponseTemplate::new(200).set_body_json(body)).await;
        let games = fetch_owned("test-key", STEAM_ID, &server.uri())
            .await
            .unwrap();
        assert!(games.is_empty());
    }

    #[tokio::test]
    async fn empty_response_means_private_profile() {
        let body = serde_json::json!({"response": {}});
        let server = server_with(ResponseTemplate::new(200).set_body_json(body)).await;
        let result = fetch_owned("test-key", STEAM_ID, &server.uri()).await;
        assert!(matches!(result, Err(OwnedError::PrivateProfile)));
    }

    #[tokio::test]
    async fn forbidden_means_bad_key() {
        let server = server_with(ResponseTemplate::new(403)).await;
        let result = fetch_owned("test-key", STEAM_ID, &server.uri()).await;
        assert!(matches!(result, Err(OwnedError::BadKey(403))));
    }

    #[tokio::test]
    async fn server_error_is_a_status_error() {
        let server = server_with(ResponseTemplate::new(500)).await;
        let result = fetch_owned("test-key", STEAM_ID, &server.uri()).await;
        assert!(matches!(result, Err(OwnedError::Status(500))));
    }

    #[tokio::test]
    async fn request_error_does_not_show_the_key() {
        // Port 9 (discard) on localhost has no listener in the test runner.
        let result = fetch_owned("secret-key", STEAM_ID, "http://127.0.0.1:9").await;
        let err = result.unwrap_err();
        assert!(matches!(err, OwnedError::Request(_)));
        assert!(!err.to_string().contains("secret-key"), "{err}");
    }

    #[test]
    fn merge_keeps_installed_and_adds_owned_only() {
        let installed = vec![entry(440, "Team Fortress 2", true)];
        let owned = vec![
            entry(440, "Team Fortress 2", false),
            entry(620, "Portal 2", false),
        ];
        assert_eq!(
            merge(installed, owned),
            vec![
                entry(620, "Portal 2", false),
                entry(440, "Team Fortress 2", true),
            ]
        );
    }

    #[test]
    fn merge_with_no_owned_games_keeps_installed() {
        let installed = vec![entry(440, "Team Fortress 2", true)];
        assert_eq!(merge(installed.clone(), Vec::new()), installed);
    }

    #[test]
    fn merge_with_nothing_installed_lists_owned() {
        let owned = vec![entry(620, "Portal 2", false)];
        assert_eq!(merge(Vec::new(), owned.clone()), owned);
    }
}
