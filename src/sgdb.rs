//! SteamGridDB client: find art for a library entry and download it.

use image::DynamicImage;
use serde::Deserialize;

use crate::assets::AssetKind;
use crate::imaging;
use crate::steam::library::{EntrySource, LibraryEntry};

pub const SGDB_BASE: &str = "https://www.steamgriddb.com/api/v2";

/// The number of images one API page holds. The API docs do not state it;
/// a page with fewer images is taken as the last page.
pub const SGDB_PAGE_SIZE: usize = 50;

#[derive(Debug, thiserror::Error)]
pub enum SgdbError {
    #[error("SteamGridDB rejected the API key (HTTP {0})")]
    BadKey(u16),
    #[error("SteamGridDB returned HTTP {0}")]
    Status(u16),
    #[error("the SteamGridDB base URL is not valid: {0}")]
    BaseUrl(String),
    #[error("the SteamGridDB request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("the downloaded image is not valid: {0}")]
    Image(#[from] image::ImageError),
}

/// One image SteamGridDB offers for a slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub id: u64,
    pub thumb_url: String,
    pub full_url: String,
}

pub struct SgdbClient {
    http: reqwest::Client,
    base_url: String,
    key: String,
}

impl SgdbClient {
    /// `base_url` is [`SGDB_BASE`] outside tests.
    pub fn new(key: &str, base_url: &str) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
            key: key.to_string(),
        }
    }

    /// The first API page of images for `kind` of `entry`.
    pub async fn candidates(
        &self,
        entry: &LibraryEntry,
        kind: AssetKind,
    ) -> Result<Vec<Candidate>, SgdbError> {
        self.candidates_page(entry, kind, 0).await
    }

    /// One API page (from 0) of images for `kind` of `entry`. A Steam game
    /// uses its app ID. A non-Steam shortcut uses the first game a name
    /// search finds. No match gives an empty list.
    pub async fn candidates_page(
        &self,
        entry: &LibraryEntry,
        kind: AssetKind,
        page: u32,
    ) -> Result<Vec<Candidate>, SgdbError> {
        // SteamGridDB has one ID space for Steam app IDs and one for its own game IDs.
        let (space, id) = match entry.source {
            EntrySource::Steam => ("steam", u64::from(entry.appid)),
            EntrySource::NonSteam => match self.search(&entry.name).await? {
                Some(id) => ("game", id),
                None => return Ok(Vec::new()),
            },
        };
        let (endpoint, dimensions) = match kind {
            AssetKind::Portrait => ("grids", Some("600x900")),
            AssetKind::Wide => ("grids", Some("920x430,460x215")),
            AssetKind::Hero => ("heroes", None),
            AssetKind::Logo => ("logos", None),
        };
        let mut request = self
            .http
            .get(format!("{}/{endpoint}/{space}/{id}", self.base_url));
        if let Some(dimensions) = dimensions {
            request = request.query(&[("dimensions", dimensions)]);
        }
        request = request.query(&[("page", page)]);
        let images: Option<Vec<SgdbImage>> = self.get_data(request).await?;
        Ok(images
            .unwrap_or_default()
            .into_iter()
            .map(|image| Candidate {
                id: image.id,
                thumb_url: image.thumb,
                full_url: image.url,
            })
            .collect())
    }

    /// Downloads the full image of `candidate` and makes it ready to
    /// write as `kind` (see [`imaging::prepare`]).
    pub async fn download(
        &self,
        candidate: &Candidate,
        kind: AssetKind,
    ) -> Result<DynamicImage, SgdbError> {
        let bytes = self.fetch_cdn(&candidate.full_url).await?;
        let image = image::load_from_memory(&bytes)?;
        Ok(imaging::prepare(&image, kind))
    }

    /// The encoded bytes of the thumbnail of `candidate`.
    pub async fn thumbnail(&self, candidate: &Candidate) -> Result<Vec<u8>, SgdbError> {
        self.fetch_cdn(&candidate.thumb_url).await
    }

    /// The bytes at `url`.
    async fn fetch_cdn(&self, url: &str) -> Result<Vec<u8>, SgdbError> {
        // Image URLs point at the SteamGridDB CDN, which needs no key.
        let response = self.http.get(url).send().await?;
        if !response.status().is_success() {
            return Err(SgdbError::Status(response.status().as_u16()));
        }
        Ok(response.bytes().await?.to_vec())
    }

    /// The ID of the first game a name search finds.
    async fn search(&self, name: &str) -> Result<Option<u64>, SgdbError> {
        let mut url = reqwest::Url::parse(&format!("{}/search/autocomplete", self.base_url))
            .map_err(|err| SgdbError::BaseUrl(err.to_string()))?;
        url.path_segments_mut()
            .map_err(|()| SgdbError::BaseUrl(self.base_url.clone()))?
            .push(name);
        let games: Option<Vec<SgdbGame>> = self.get_data(self.http.get(url)).await?;
        Ok(games.unwrap_or_default().first().map(|game| game.id))
    }

    /// Sends `request` with the key and returns the `data` field.
    /// HTTP 404 (SteamGridDB does not know the game) gives `None`.
    async fn get_data<T: serde::de::DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<Option<T>, SgdbError> {
        let response = request.bearer_auth(&self.key).send().await?;
        let status = response.status().as_u16();
        match status {
            200..=299 => {}
            401 | 403 => return Err(SgdbError::BadKey(status)),
            404 => return Ok(None),
            _ => return Err(SgdbError::Status(status)),
        }
        let body: DataBody<T> = response.json().await?;
        Ok(Some(body.data))
    }
}

#[derive(Deserialize)]
struct DataBody<T> {
    data: T,
}

#[derive(Deserialize)]
struct SgdbImage {
    id: u64,
    url: String,
    thumb: String,
}

#[derive(Deserialize)]
struct SgdbGame {
    id: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, RgbaImage};
    use std::io::Cursor;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const KEY: &str = "test-key";

    fn steam_entry(appid: u32) -> LibraryEntry {
        LibraryEntry {
            appid,
            name: "Team Fortress 2".to_string(),
            source: EntrySource::Steam,
            installed: true,
        }
    }

    fn shortcut_entry(name: &str) -> LibraryEntry {
        LibraryEntry {
            appid: 3_221_225_472,
            name: name.to_string(),
            source: EntrySource::NonSteam,
            installed: true,
        }
    }

    /// A SteamGridDB list body with one image per ID.
    fn images_body(server: &MockServer, ids: &[u64]) -> serde_json::Value {
        let data: Vec<_> = ids
            .iter()
            .map(|id| {
                serde_json::json!({
                    "id": id,
                    "score": 0,
                    "style": "alternate",
                    "width": 600,
                    "height": 900,
                    "nsfw": false,
                    "humor": false,
                    "mime": "image/png",
                    "url": format!("{}/full/{id}.png", server.uri()),
                    "thumb": format!("{}/thumb/{id}.png", server.uri()),
                })
            })
            .collect();
        serde_json::json!({"success": true, "data": data})
    }

    fn candidate(server: &MockServer, id: u64) -> Candidate {
        Candidate {
            id,
            thumb_url: format!("{}/thumb/{id}.png", server.uri()),
            full_url: format!("{}/full/{id}.png", server.uri()),
        }
    }

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let image = RgbaImage::from_pixel(width, height, image::Rgba([10, 20, 30, 255]));
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    async fn mount_list(server: &MockServer, list_path: &str, body: serde_json::Value) {
        Mock::given(method("GET"))
            .and(path(list_path))
            .and(header("Authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn portrait_asks_for_600x900_grids() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/grids/steam/440"))
            .and(query_param("dimensions", "600x900"))
            .and(header("Authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(images_body(&server, &[1, 2])))
            .expect(1)
            .mount(&server)
            .await;
        let client = SgdbClient::new(KEY, &server.uri());
        let found = client
            .candidates(&steam_entry(440), AssetKind::Portrait)
            .await
            .unwrap();
        assert_eq!(found, vec![candidate(&server, 1), candidate(&server, 2)]);
    }

    #[tokio::test]
    async fn wide_asks_for_wide_grids() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/grids/steam/440"))
            .and(query_param("dimensions", "920x430,460x215"))
            .and(header("Authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(images_body(&server, &[3])))
            .expect(1)
            .mount(&server)
            .await;
        let client = SgdbClient::new(KEY, &server.uri());
        let found = client
            .candidates(&steam_entry(440), AssetKind::Wide)
            .await
            .unwrap();
        assert_eq!(found, vec![candidate(&server, 3)]);
    }

    #[tokio::test]
    async fn hero_uses_heroes_endpoint() {
        let server = MockServer::start().await;
        mount_list(&server, "/heroes/steam/440", images_body(&server, &[4])).await;
        let client = SgdbClient::new(KEY, &server.uri());
        let found = client
            .candidates(&steam_entry(440), AssetKind::Hero)
            .await
            .unwrap();
        assert_eq!(found, vec![candidate(&server, 4)]);
    }

    #[tokio::test]
    async fn logo_uses_logos_endpoint() {
        let server = MockServer::start().await;
        mount_list(&server, "/logos/steam/440", images_body(&server, &[5])).await;
        let client = SgdbClient::new(KEY, &server.uri());
        let found = client
            .candidates(&steam_entry(440), AssetKind::Logo)
            .await
            .unwrap();
        assert_eq!(found, vec![candidate(&server, 5)]);
    }

    #[tokio::test]
    async fn non_steam_entry_searches_then_uses_first_game_id() {
        let server = MockServer::start().await;
        let search = serde_json::json!({
            "success": true,
            "data": [
                {"id": 1234, "name": "My Game", "types": [], "verified": true},
                {"id": 5678, "name": "My Game 2", "types": [], "verified": false}
            ]
        });
        // The name holds a space and a slash, so it must be one encoded segment.
        mount_list(&server, "/search/autocomplete/My%20Game%2F2", search).await;
        Mock::given(method("GET"))
            .and(path("/grids/game/1234"))
            .and(query_param("dimensions", "600x900"))
            .and(header("Authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(images_body(&server, &[6])))
            .expect(1)
            .mount(&server)
            .await;
        let client = SgdbClient::new(KEY, &server.uri());
        let found = client
            .candidates(&shortcut_entry("My Game/2"), AssetKind::Portrait)
            .await
            .unwrap();
        assert_eq!(found, vec![candidate(&server, 6)]);
    }

    #[tokio::test]
    async fn non_steam_search_with_no_match_gives_no_candidates() {
        let server = MockServer::start().await;
        let search = serde_json::json!({"success": true, "data": []});
        mount_list(&server, "/search/autocomplete/Unknown", search).await;
        let client = SgdbClient::new(KEY, &server.uri());
        let found = client
            .candidates(&shortcut_entry("Unknown"), AssetKind::Hero)
            .await
            .unwrap();
        assert!(found.is_empty());
    }

    #[tokio::test]
    async fn empty_list_gives_no_candidates() {
        let server = MockServer::start().await;
        mount_list(&server, "/heroes/steam/440", images_body(&server, &[])).await;
        let client = SgdbClient::new(KEY, &server.uri());
        let found = client
            .candidates(&steam_entry(440), AssetKind::Hero)
            .await
            .unwrap();
        assert!(found.is_empty());
    }

    #[tokio::test]
    async fn unknown_game_gives_no_candidates() {
        let server = MockServer::start().await;
        let body = serde_json::json!({"success": false, "errors": ["Game not found"]});
        Mock::given(method("GET"))
            .and(path("/logos/steam/999999"))
            .respond_with(ResponseTemplate::new(404).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
        let client = SgdbClient::new(KEY, &server.uri());
        let found = client
            .candidates(&steam_entry(999_999), AssetKind::Logo)
            .await
            .unwrap();
        assert!(found.is_empty());
    }

    #[tokio::test]
    async fn unauthorized_means_bad_key() {
        let server = MockServer::start().await;
        let body = serde_json::json!({"success": false, "errors": ["Unauthorized"]});
        Mock::given(method("GET"))
            .and(path("/heroes/steam/440"))
            .respond_with(ResponseTemplate::new(401).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
        let client = SgdbClient::new("bad-key", &server.uri());
        let result = client.candidates(&steam_entry(440), AssetKind::Hero).await;
        assert!(matches!(result, Err(SgdbError::BadKey(401))), "{result:?}");
    }

    #[tokio::test]
    async fn unauthorized_search_means_bad_key() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/search/autocomplete/My%20Game"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        let client = SgdbClient::new("bad-key", &server.uri());
        let result = client
            .candidates(&shortcut_entry("My Game"), AssetKind::Hero)
            .await;
        assert!(matches!(result, Err(SgdbError::BadKey(401))), "{result:?}");
    }

    #[tokio::test]
    async fn server_error_is_a_status_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/heroes/steam/440"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;
        let client = SgdbClient::new(KEY, &server.uri());
        let result = client.candidates(&steam_entry(440), AssetKind::Hero).await;
        assert!(matches!(result, Err(SgdbError::Status(500))), "{result:?}");
    }

    #[tokio::test]
    async fn candidates_page_asks_for_that_api_page() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/heroes/steam/440"))
            .and(query_param("page", "2"))
            .and(header("Authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(images_body(&server, &[21])))
            .expect(1)
            .mount(&server)
            .await;
        let client = SgdbClient::new(KEY, &server.uri());
        let found = client
            .candidates_page(&steam_entry(440), AssetKind::Hero, 2)
            .await
            .unwrap();
        assert_eq!(found, vec![candidate(&server, 21)]);
    }

    #[tokio::test]
    async fn candidates_asks_for_the_first_api_page() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/logos/steam/440"))
            .and(query_param("page", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(images_body(&server, &[22])))
            .expect(1)
            .mount(&server)
            .await;
        let client = SgdbClient::new(KEY, &server.uri());
        let found = client
            .candidates(&steam_entry(440), AssetKind::Logo)
            .await
            .unwrap();
        assert_eq!(found, vec![candidate(&server, 22)]);
    }

    #[tokio::test]
    async fn thumbnail_gives_the_thumb_bytes_without_the_key() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/thumb/23.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"thumb".to_vec()))
            .expect(1)
            .mount(&server)
            .await;
        let client = SgdbClient::new(KEY, &server.uri());
        let bytes = client.thumbnail(&candidate(&server, 23)).await.unwrap();
        assert_eq!(bytes, b"thumb");
        let requests = server.received_requests().await.unwrap();
        assert!(requests[0].headers.get("authorization").is_none());
    }

    #[tokio::test]
    async fn thumbnail_of_a_missing_image_is_a_status_error() {
        let server = MockServer::start().await;
        let client = SgdbClient::new(KEY, &server.uri());
        let result = client.thumbnail(&candidate(&server, 24)).await;
        assert!(matches!(result, Err(SgdbError::Status(404))), "{result:?}");
    }

    #[tokio::test]
    async fn download_fits_the_image_to_the_steam_size() {
        let server = MockServer::start().await;
        // SteamGridDB grids can be a little off the exact Steam size.
        Mock::given(method("GET"))
            .and(path("/full/7.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(png_bytes(602, 900)))
            .expect(1)
            .mount(&server)
            .await;
        let client = SgdbClient::new(KEY, &server.uri());
        let image = client
            .download(&candidate(&server, 7), AssetKind::Portrait)
            .await
            .unwrap();
        assert_eq!((image.width(), image.height()), (600, 900));
    }

    #[tokio::test]
    async fn download_does_not_send_the_key() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/full/8.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(png_bytes(1920, 620)))
            .expect(1)
            .mount(&server)
            .await;
        let client = SgdbClient::new(KEY, &server.uri());
        client
            .download(&candidate(&server, 8), AssetKind::Hero)
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        assert!(requests[0].headers.get("authorization").is_none());
    }

    #[tokio::test]
    async fn download_of_a_missing_image_is_a_status_error() {
        let server = MockServer::start().await;
        let client = SgdbClient::new(KEY, &server.uri());
        let result = client
            .download(&candidate(&server, 9), AssetKind::Wide)
            .await;
        assert!(matches!(result, Err(SgdbError::Status(404))), "{result:?}");
    }

    #[tokio::test]
    async fn download_of_bad_bytes_is_an_image_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/full/10.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"not an image".to_vec()))
            .mount(&server)
            .await;
        let client = SgdbClient::new(KEY, &server.uri());
        let result = client
            .download(&candidate(&server, 10), AssetKind::Wide)
            .await;
        assert!(matches!(result, Err(SgdbError::Image(_))), "{result:?}");
    }
}
