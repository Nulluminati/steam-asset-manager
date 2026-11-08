//! Steam's own library art for a Steam game: the local library cache
//! first, then the Steam CDN.

use std::path::{Path, PathBuf};

use crate::assets::AssetKind;

use super::library::EntrySource;

/// The Steam CDN folder that holds the store art of every app.
pub const STEAM_CDN_BASE: &str =
    "https://shared.cloudflare.steamstatic.com/store_item_assets/steam/apps";

#[derive(Debug, thiserror::Error)]
pub enum DefaultsError {
    #[error("the Steam CDN returned HTTP {0}")]
    Status(u16),
    #[error("the Steam CDN request failed: {0}")]
    Request(#[from] reqwest::Error),
}

/// Steam's own art for one slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultArt {
    /// A file in the local library cache.
    Cached(PathBuf),
    /// The file body from the Steam CDN.
    Downloaded(Vec<u8>),
}

/// The library cache folder in the Steam root.
pub fn library_cache_dir(steam_root: &Path) -> PathBuf {
    steam_root.join("appcache").join("librarycache")
}

/// The file name Steam uses for the default art of `kind`, in the cache
/// and on the CDN.
pub fn default_file_name(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Portrait => "library_600x900.jpg",
        AssetKind::Wide => "header.jpg",
        AssetKind::Hero => "library_hero.jpg",
        AssetKind::Logo => "logo.png",
    }
}

/// Finds the cached default art for one slot. Supports the flat layout
/// (`<appid>_library_600x900.jpg`) and the per-app folder layout
/// (`<appid>/library_600x900.jpg`, or one folder deeper in a folder with a
/// content-hash name).
pub fn find_cached(cache_dir: &Path, appid: u32, kind: AssetKind) -> Option<PathBuf> {
    let name = default_file_name(kind);
    let flat = cache_dir.join(format!("{appid}_{name}"));
    if flat.is_file() {
        return Some(flat);
    }

    let app_dir = cache_dir.join(appid.to_string());
    let direct = app_dir.join(name);
    if direct.is_file() {
        return Some(direct);
    }

    // Sorted, so the result does not depend on the order the OS lists them.
    let mut subfolders: Vec<PathBuf> = std::fs::read_dir(&app_dir)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.is_dir())
        .collect();
    subfolders.sort();
    subfolders
        .into_iter()
        .map(|folder| folder.join(name))
        .find(|path| path.is_file())
}

/// Downloads the default art for one slot from the CDN. Returns `None` when
/// the CDN has no such file. `base_url` is [`STEAM_CDN_BASE`] outside tests.
pub async fn fetch_cdn(
    base_url: &str,
    appid: u32,
    kind: AssetKind,
) -> Result<Option<Vec<u8>>, DefaultsError> {
    let url = format!(
        "{}/{appid}/{}",
        base_url.trim_end_matches('/'),
        default_file_name(kind)
    );
    let response = reqwest::get(url).await?;
    let status = response.status().as_u16();
    match status {
        200..=299 => Ok(Some(response.bytes().await?.to_vec())),
        404 => Ok(None),
        _ => Err(DefaultsError::Status(status)),
    }
}

/// Steam's own art for one slot: the cached file, else the CDN file.
/// Non-Steam shortcuts have no default art.
pub async fn default_art(
    cache_dir: &Path,
    base_url: &str,
    appid: u32,
    source: EntrySource,
    kind: AssetKind,
) -> Result<Option<DefaultArt>, DefaultsError> {
    if source == EntrySource::NonSteam {
        return Ok(None);
    }
    if let Some(path) = find_cached(cache_dir, appid, kind) {
        return Ok(Some(DefaultArt::Cached(path)));
    }
    Ok(fetch_cdn(base_url, appid, kind)
        .await?
        .map(DefaultArt::Downloaded))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const HASH_FOLDER: &str = "3f9c1a7e5b2d4c6a8e0f1b3d5a7c9e2f4b6d8a0c";

    fn fixture_cache() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/steam/appcache/librarycache")
    }

    async fn cdn_with(file_path: &str, response: ResponseTemplate, calls: u64) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(file_path))
            .respond_with(response)
            .expect(calls)
            .mount(&server)
            .await;
        server
    }

    #[test]
    fn cache_dir_is_in_appcache() {
        let root = Path::new("steam");
        assert_eq!(
            library_cache_dir(root),
            root.join("appcache").join("librarycache")
        );
    }

    #[test]
    fn file_names() {
        assert_eq!(
            default_file_name(AssetKind::Portrait),
            "library_600x900.jpg"
        );
        assert_eq!(default_file_name(AssetKind::Wide), "header.jpg");
        assert_eq!(default_file_name(AssetKind::Hero), "library_hero.jpg");
        assert_eq!(default_file_name(AssetKind::Logo), "logo.png");
    }

    #[test]
    fn cache_hit_flat_layout() {
        let cache = fixture_cache();
        for (kind, name) in [
            (AssetKind::Portrait, "440_library_600x900.jpg"),
            (AssetKind::Wide, "440_header.jpg"),
            (AssetKind::Hero, "440_library_hero.jpg"),
            (AssetKind::Logo, "440_logo.png"),
        ] {
            assert_eq!(
                find_cached(&cache, 440, kind),
                Some(cache.join(name)),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn cache_hit_app_folder_layout() {
        let cache = fixture_cache();
        let app = cache.join("620");
        assert_eq!(
            find_cached(&cache, 620, AssetKind::Wide),
            Some(app.join("header.jpg"))
        );
        assert_eq!(
            find_cached(&cache, 620, AssetKind::Hero),
            Some(app.join("library_hero.jpg"))
        );
        assert_eq!(
            find_cached(&cache, 620, AssetKind::Logo),
            Some(app.join("logo.png"))
        );
    }

    #[test]
    fn cache_hit_in_hash_folder() {
        let cache = fixture_cache();
        assert_eq!(
            find_cached(&cache, 620, AssetKind::Portrait),
            Some(
                cache
                    .join("620")
                    .join(HASH_FOLDER)
                    .join("library_600x900.jpg")
            )
        );
    }

    #[test]
    fn app_folder_wins_over_hash_folder() {
        let temp = TempDir::new().unwrap();
        let app = temp.path().join("620");
        std::fs::create_dir_all(app.join("abc")).unwrap();
        std::fs::write(app.join("abc").join("logo.png"), b"old").unwrap();
        std::fs::write(app.join("logo.png"), b"new").unwrap();
        assert_eq!(
            find_cached(temp.path(), 620, AssetKind::Logo),
            Some(app.join("logo.png"))
        );
    }

    #[test]
    fn cache_miss() {
        let cache = fixture_cache();
        assert_eq!(find_cached(&cache, 400, AssetKind::Hero), None);
        let temp = TempDir::new().unwrap();
        assert_eq!(
            find_cached(&temp.path().join("missing"), 440, AssetKind::Hero),
            None
        );
    }

    #[tokio::test]
    async fn cdn_download() {
        let server = cdn_with(
            "/400/library_hero.jpg",
            ResponseTemplate::new(200).set_body_bytes(b"hero".to_vec()),
            1,
        )
        .await;
        let body = fetch_cdn(&server.uri(), 400, AssetKind::Hero)
            .await
            .unwrap();
        assert_eq!(body, Some(b"hero".to_vec()));
    }

    #[tokio::test]
    async fn cdn_not_found_is_none() {
        let server = cdn_with("/400/logo.png", ResponseTemplate::new(404), 1).await;
        let body = fetch_cdn(&server.uri(), 400, AssetKind::Logo)
            .await
            .unwrap();
        assert_eq!(body, None);
    }

    #[tokio::test]
    async fn cdn_server_error() {
        let server = cdn_with("/400/header.jpg", ResponseTemplate::new(503), 1).await;
        let result = fetch_cdn(&server.uri(), 400, AssetKind::Wide).await;
        assert!(
            matches!(result, Err(DefaultsError::Status(503))),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn default_art_uses_cache_first() {
        let server = cdn_with("/440/header.jpg", ResponseTemplate::new(200), 0).await;
        let cache = fixture_cache();
        let art = default_art(
            &cache,
            &server.uri(),
            440,
            EntrySource::Steam,
            AssetKind::Wide,
        )
        .await
        .unwrap();
        assert_eq!(art, Some(DefaultArt::Cached(cache.join("440_header.jpg"))));
    }

    #[tokio::test]
    async fn default_art_falls_back_to_cdn() {
        let server = cdn_with(
            "/400/library_600x900.jpg",
            ResponseTemplate::new(200).set_body_bytes(b"portrait".to_vec()),
            1,
        )
        .await;
        let art = default_art(
            &fixture_cache(),
            &server.uri(),
            400,
            EntrySource::Steam,
            AssetKind::Portrait,
        )
        .await
        .unwrap();
        assert_eq!(art, Some(DefaultArt::Downloaded(b"portrait".to_vec())));
    }

    #[tokio::test]
    async fn default_art_is_none_when_cdn_has_none() {
        let server = cdn_with("/400/logo.png", ResponseTemplate::new(404), 1).await;
        let art = default_art(
            &fixture_cache(),
            &server.uri(),
            400,
            EntrySource::Steam,
            AssetKind::Logo,
        )
        .await
        .unwrap();
        assert_eq!(art, None);
    }

    #[tokio::test]
    async fn non_steam_entry_has_no_default_art() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;
        let art = default_art(
            &fixture_cache(),
            &server.uri(),
            440,
            EntrySource::NonSteam,
            AssetKind::Hero,
        )
        .await
        .unwrap();
        assert_eq!(art, None);
    }
}
