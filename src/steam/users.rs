use std::path::Path;

use super::{SteamError, get_str, parse_vdf, read_text};

/// Account ID = SteamID64 - this value.
pub const STEAM_ID64_BASE: u64 = 76_561_197_960_265_728;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SteamUser {
    pub steam_id64: u64,
    /// The name of the user folder under `<root>/userdata/`.
    pub account_id: u32,
}

/// Parses the text of `loginusers.vdf` and returns the user with
/// `MostRecent "1"`, or `None` when no user has it.
pub fn parse_most_recent_user(text: &str) -> Result<Option<SteamUser>, SteamError> {
    let users = parse_vdf(text, "loginusers.vdf")?;
    for (key, values) in users.iter() {
        let Some(user) = values.first().and_then(|value| value.get_obj()) else {
            continue;
        };
        if get_str(user, "MostRecent") != Some("1") {
            continue;
        }
        let Some(steam_user) = steam_user_from_id64(key) else {
            continue;
        };
        return Ok(Some(steam_user));
    }
    Ok(None)
}

fn steam_user_from_id64(key: &str) -> Option<SteamUser> {
    let steam_id64: u64 = key.parse().ok()?;
    let account_id = u32::try_from(steam_id64.checked_sub(STEAM_ID64_BASE)?).ok()?;
    Some(SteamUser {
        steam_id64,
        account_id,
    })
}

/// Reads `<root>/config/loginusers.vdf` and returns the most recent user.
pub fn most_recent_user(steam_root: &Path) -> Result<Option<SteamUser>, SteamError> {
    let text = read_text(&steam_root.join("config").join("loginusers.vdf"))?;
    parse_most_recent_user(&text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/steam")
    }

    #[test]
    fn one_user_with_most_recent() {
        let text = r#"
"users"
{
    "76561197960287930"
    {
        "AccountName"    "zerocool"
        "MostRecent"     "1"
    }
}
"#;
        assert_eq!(
            parse_most_recent_user(text).unwrap(),
            Some(SteamUser {
                steam_id64: 76_561_197_960_287_930,
                account_id: 22_202,
            })
        );
    }

    #[test]
    fn several_users_from_fixture() {
        assert_eq!(
            most_recent_user(&fixture_root()).unwrap(),
            Some(SteamUser {
                steam_id64: 76_561_198_000_000_001,
                account_id: 39_734_273,
            })
        );
    }

    #[test]
    fn no_most_recent_gives_none() {
        let text = r#"
"users"
{
    "76561197960287930"
    {
        "AccountName"    "zerocool"
        "MostRecent"     "0"
    }
    "76561198000000001"
    {
        "AccountName"    "acidburn"
    }
}
"#;
        assert_eq!(parse_most_recent_user(text).unwrap(), None);
    }

    #[test]
    fn lower_case_most_recent_key() {
        let text = r#"
"users"
{
    "76561197960287930"
    {
        "mostrecent"     "1"
    }
}
"#;
        assert_eq!(
            parse_most_recent_user(text).unwrap().map(|u| u.account_id),
            Some(22_202)
        );
    }

    #[test]
    fn invalid_vdf_is_an_error() {
        assert!(matches!(
            parse_most_recent_user("\"users\" {"),
            Err(SteamError::Parse { .. })
        ));
    }

    #[test]
    fn missing_file_is_an_error() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(matches!(
            most_recent_user(dir.path()),
            Err(SteamError::Io { .. })
        ));
    }
}
