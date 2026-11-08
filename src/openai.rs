//! OpenAI image client: make Steam library art with the image API.

use std::io::Cursor;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use image::{DynamicImage, ImageFormat};
use serde::Deserialize;

use crate::assets::AssetKind;
use crate::imaging;

pub const OPENAI_BASE: &str = "https://api.openai.com";
/// Image calls can take one to two minutes.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(300);
/// How many images one `generate` or `modify` call asks for.
pub const CANDIDATE_COUNT: u8 = 4;

#[derive(Debug, thiserror::Error)]
pub enum OpenAiError {
    #[error("OpenAI rejected the API key (HTTP {0})")]
    BadKey(u16),
    #[error("OpenAI returned HTTP {status}: {message}")]
    Api { status: u16, message: String },
    #[error("the OpenAI request failed: {0}")]
    Request(#[from] reqwest::Error),
    #[error("the OpenAI image data is not valid base64: {0}")]
    Decode(#[from] base64::DecodeError),
    #[error("the OpenAI image is not valid: {0}")]
    Image(#[from] image::ImageError),
    #[error("an OpenAI result item has no image data")]
    MissingImage,
}

/// The custom size to request for `kind`. Every size obeys OpenAI's rules:
/// a multiple of 16 on each edge, a ratio within 1:3 to 3:1, at most
/// 8,294,400 pixels and no edge above 3840.
pub fn request_size(kind: AssetKind) -> (u32, u32) {
    match kind {
        AssetKind::Portrait => (832, 1248),
        AssetKind::Wide => (1472, 688),
        AssetKind::Hero => (1920, 640),
        AssetKind::Logo => (1536, 1024),
    }
}

/// The `size` field, for example `832x1248`.
fn size_param(kind: AssetKind) -> String {
    let (width, height) = request_size(kind);
    format!("{width}x{height}")
}

/// Extra words that tell the model what the slot needs.
fn kind_hint(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Portrait => "Vertical game cover art, 2:3 portrait, the game title on the art.",
        AssetKind::Wide => "Wide game banner art, the game title on the art.",
        AssetKind::Hero => "Wide cinematic background scene, no text, no logo.",
        AssetKind::Logo => "The game title as a logo only, on a transparent background, no scene.",
    }
}

/// The user text, then the hint for `kind`. Blank user text gives the
/// hint alone.
pub fn build_prompt(user: &str, kind: AssetKind) -> String {
    let user = user.trim();
    if user.is_empty() {
        return kind_hint(kind).to_string();
    }
    format!("{user}\n\n{}", kind_hint(kind))
}

pub struct OpenAiClient {
    http: reqwest::Client,
    base_url: String,
    key: String,
    model: String,
}

impl OpenAiClient {
    /// `base_url` is [`OPENAI_BASE`] outside tests.
    pub fn new(key: &str, model: &str, base_url: &str) -> Self {
        Self::with_timeout(key, model, base_url, REQUEST_TIMEOUT)
    }

    /// Like [`OpenAiClient::new`], with a request timeout of your choice.
    ///
    /// # Panics
    ///
    /// Panics if the HTTP client cannot be built.
    pub fn with_timeout(key: &str, model: &str, base_url: &str, timeout: Duration) -> Self {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .expect("cannot build the HTTP client");
        Self {
            http,
            base_url: base_url.trim_end_matches('/').to_string(),
            key: key.to_string(),
            model: model.to_string(),
        }
    }

    /// Makes four new images from `prompt`, ready to write as `kind`.
    pub async fn generate(
        &self,
        prompt: &str,
        kind: AssetKind,
    ) -> Result<Vec<DynamicImage>, OpenAiError> {
        let mut body = serde_json::json!({
            "model": self.model,
            "prompt": build_prompt(prompt, kind),
            "n": CANDIDATE_COUNT,
            "size": size_param(kind),
            "output_format": "png",
        });
        if kind == AssetKind::Logo {
            body["background"] = serde_json::json!("transparent");
        }
        let response = self
            .http
            .post(format!("{}/v1/images/generations", self.base_url))
            .bearer_auth(&self.key)
            .json(&body)
            .send()
            .await?;
        self.images_from(response, kind).await
    }

    /// Edits `source` with `prompt` and returns four images ready to
    /// write as `kind`.
    pub async fn modify(
        &self,
        source: &DynamicImage,
        prompt: &str,
        kind: AssetKind,
    ) -> Result<Vec<DynamicImage>, OpenAiError> {
        let mut png = Cursor::new(Vec::new());
        source.write_to(&mut png, ImageFormat::Png)?;
        let image_part = reqwest::multipart::Part::bytes(png.into_inner())
            .file_name("source.png")
            .mime_str("image/png")?;
        let mut form = reqwest::multipart::Form::new()
            .text("model", self.model.clone())
            .text("prompt", build_prompt(prompt, kind))
            .text("n", CANDIDATE_COUNT.to_string())
            .text("size", size_param(kind))
            .text("output_format", "png")
            .part("image", image_part);
        if kind == AssetKind::Logo {
            form = form.text("background", "transparent");
        }
        let response = self
            .http
            .post(format!("{}/v1/images/edits", self.base_url))
            .bearer_auth(&self.key)
            .multipart(form)
            .send()
            .await?;
        self.images_from(response, kind).await
    }

    /// Maps the status and turns every result item into an image.
    async fn images_from(
        &self,
        response: reqwest::Response,
        kind: AssetKind,
    ) -> Result<Vec<DynamicImage>, OpenAiError> {
        let status = response.status().as_u16();
        if status == 401 {
            return Err(OpenAiError::BadKey(status));
        }
        if !response.status().is_success() {
            let body = response.text().await?;
            return Err(OpenAiError::Api {
                status,
                message: message_from_body(&body),
            });
        }
        let body: ImagesBody = response.json().await?;
        body.data
            .iter()
            .map(|item| decode_item(item, kind))
            .collect()
    }
}

/// OpenAI's own reason for a failed call, or the body text as it is.
fn message_from_body(body: &str) -> String {
    match serde_json::from_str::<ErrorBody>(body) {
        Ok(parsed) => parsed.error.message,
        Err(_) => body.to_string(),
    }
}

fn decode_item(item: &ImageItem, kind: AssetKind) -> Result<DynamicImage, OpenAiError> {
    let data = item.b64_json.as_deref().ok_or(OpenAiError::MissingImage)?;
    let bytes = STANDARD.decode(data)?;
    let image = image::load_from_memory(&bytes)?;
    Ok(imaging::prepare(&image, kind))
}

#[derive(Deserialize)]
struct ImagesBody {
    data: Vec<ImageItem>,
}

#[derive(Deserialize)]
struct ImageItem {
    b64_json: Option<String>,
}

#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorMessage,
}

#[derive(Deserialize)]
struct ErrorMessage {
    message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD;
    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    use std::io::Cursor;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const KEY: &str = "test-key";
    const MODEL: &str = "gpt-image-2.6-sunburst";

    /// OpenAI's rules for a custom size.
    fn follows_the_size_rules((width, height): (u32, u32)) -> bool {
        let (width_64, height_64) = (u64::from(width), u64::from(height));
        let pixels = width_64 * height_64;
        // Ratio within 1:3 to 3:1.
        let ratio_in_range = height_64 <= width_64 * 3 && width_64 <= height_64 * 3;
        width % 16 == 0
            && height % 16 == 0
            && ratio_in_range
            && (655_360..=8_294_400).contains(&pixels)
            && width <= 3840
            && height <= 3840
    }

    #[test]
    fn request_size_gives_the_four_sizes() {
        assert_eq!(request_size(AssetKind::Portrait), (832, 1248));
        assert_eq!(request_size(AssetKind::Wide), (1472, 688));
        assert_eq!(request_size(AssetKind::Hero), (1920, 640));
        assert_eq!(request_size(AssetKind::Logo), (1536, 1024));
    }

    #[test]
    fn every_request_size_obeys_the_openai_rules() {
        for kind in AssetKind::ALL {
            let size = request_size(kind);
            assert!(follows_the_size_rules(size), "{kind:?}: {size:?}");
        }
    }

    #[test]
    fn portrait_and_wide_keep_the_steam_aspect_ratio() {
        for kind in [AssetKind::Portrait, AssetKind::Wide] {
            let (request_width, request_height) = request_size(kind);
            let (steam_width, steam_height) = kind.target_size().unwrap();
            assert_eq!(
                u64::from(request_width) * u64::from(steam_height),
                u64::from(request_height) * u64::from(steam_width),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn prompt_puts_the_user_text_first_and_the_hint_last() {
        for kind in AssetKind::ALL {
            let prompt = build_prompt("a lighthouse at dusk", kind);
            let hint = kind_hint(kind);
            assert!(prompt.starts_with("a lighthouse at dusk"), "{kind:?}");
            assert!(prompt.ends_with(hint), "{kind:?}");
        }
    }

    #[test]
    fn hero_hint_has_no_text_and_logo_hint_is_transparent() {
        assert!(kind_hint(AssetKind::Hero).contains("no text"));
        assert!(kind_hint(AssetKind::Logo).contains("transparent"));
    }

    #[test]
    fn blank_user_text_gives_the_hint_alone() {
        for kind in AssetKind::ALL {
            assert_eq!(build_prompt("  \n", kind), kind_hint(kind), "{kind:?}");
        }
    }

    fn solid(width: u32, height: u32) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            width,
            height,
            Rgba([10, 20, 30, 255]),
        ))
    }

    fn png_base64(image: &DynamicImage) -> String {
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();
        STANDARD.encode(bytes.into_inner())
    }

    /// An API response body with one image per entry.
    fn images_body(images: &[DynamicImage]) -> serde_json::Value {
        let data: Vec<_> = images
            .iter()
            .map(|image| serde_json::json!({"b64_json": png_base64(image)}))
            .collect();
        serde_json::json!({"data": data, "usage": {"total_tokens": 100}})
    }

    fn four(size: (u32, u32)) -> Vec<DynamicImage> {
        (0..CANDIDATE_COUNT)
            .map(|_| solid(size.0, size.1))
            .collect()
    }

    #[tokio::test]
    async fn modify_sends_multipart_parts() {
        let server = MockServer::start().await;
        let images = four((1472, 688));
        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .and(header("Authorization", "Bearer test-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(images_body(&images)))
            .expect(1)
            .mount(&server)
            .await;
        let client = OpenAiClient::new(KEY, MODEL, &server.uri());
        let source = solid(920, 430);
        let result = client
            .modify(&source, "make it brighter", AssetKind::Wide)
            .await
            .unwrap();
        assert_eq!(result.len(), 4);
        for image in &result {
            assert_eq!((image.width(), image.height()), (920, 430));
        }
        let requests = server.received_requests().await.unwrap();
        let request = &requests[0];
        let content_type = request
            .headers
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(
            content_type.starts_with("multipart/form-data"),
            "{content_type}"
        );
        let body = String::from_utf8_lossy(&request.body);
        assert!(body.contains("name=\"model\""), "{body}");
        assert!(body.contains(MODEL), "{body}");
        assert!(body.contains("name=\"prompt\""), "{body}");
        assert!(
            body.contains(&build_prompt("make it brighter", AssetKind::Wide)),
            "{body}"
        );
        assert!(body.contains("name=\"n\"\r\n\r\n4\r\n"), "{body}");
        assert!(body.contains("name=\"size\"\r\n\r\n1472x688\r\n"), "{body}");
        assert!(body.contains("name=\"output_format\""), "{body}");
        assert!(
            body.contains("name=\"image\"; filename=\"source.png\""),
            "{body}"
        );
        assert!(body.contains("Content-Type: image/png"), "{body}");
        assert!(
            request.body.windows(4).any(|window| window == b"\x89PNG"),
            "no PNG signature in the body"
        );
        assert!(!body.contains("name=\"background\""), "{body}");
    }

    #[tokio::test]
    async fn modify_logo_sends_transparent_background_part() {
        let server = MockServer::start().await;
        let logo = DynamicImage::ImageRgba8(RgbaImage::from_fn(1536, 1024, |x, y| {
            if (100..900).contains(&x) && (400..600).contains(&y) {
                Rgba([10, 20, 30, 255])
            } else {
                Rgba([0, 0, 0, 0])
            }
        }));
        let images: Vec<_> = (0..CANDIDATE_COUNT).map(|_| logo.clone()).collect();
        Mock::given(method("POST"))
            .and(path("/v1/images/edits"))
            .respond_with(ResponseTemplate::new(200).set_body_json(images_body(&images)))
            .expect(1)
            .mount(&server)
            .await;
        let client = OpenAiClient::new(KEY, MODEL, &server.uri());
        let result = client
            .modify(&solid(600, 900), "make it brighter", AssetKind::Logo)
            .await
            .unwrap();
        for image in &result {
            assert_eq!((image.width(), image.height()), (800, 200));
            assert!(image.color().has_alpha());
        }
        let requests = server.received_requests().await.unwrap();
        let body = String::from_utf8_lossy(&requests[0].body);
        assert!(
            body.contains("name=\"background\"\r\n\r\ntransparent\r\n"),
            "{body}"
        );
    }

    #[tokio::test]
    async fn unauthorized_means_bad_key() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "error": {
                "message": "Incorrect API key provided.",
                "type": "invalid_request_error",
                "param": null,
                "code": "invalid_api_key"
            }
        });
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(401).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
        let client = OpenAiClient::new("bad-key", MODEL, &server.uri());
        let result = client
            .generate("a lighthouse at dusk", AssetKind::Hero)
            .await;
        assert!(
            matches!(result, Err(OpenAiError::BadKey(401))),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn api_error_body_gives_its_message() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "error": {
                "message": "Your request was rejected by the safety system.",
                "type": "invalid_request_error",
                "param": null,
                "code": "moderation_blocked"
            }
        });
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(400).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
        let client = OpenAiClient::new(KEY, MODEL, &server.uri());
        let result = client
            .generate("a lighthouse at dusk", AssetKind::Hero)
            .await;
        match result {
            Err(OpenAiError::Api { status, message }) => {
                assert_eq!(status, 400);
                assert_eq!(message, "Your request was rejected by the safety system.");
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn non_json_error_body_gives_the_raw_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(502).set_body_string("bad gateway"))
            .expect(1)
            .mount(&server)
            .await;
        let client = OpenAiClient::new(KEY, MODEL, &server.uri());
        let result = client
            .generate("a lighthouse at dusk", AssetKind::Hero)
            .await;
        match result {
            Err(OpenAiError::Api { status, message }) => {
                assert_eq!(status, 502);
                assert_eq!(message, "bad gateway");
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn forbidden_gives_the_api_message() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "error": {
                "message": "The model `gpt-image-2.6-sunburst` is not enabled for this organization.",
                "type": "invalid_request_error",
                "param": null,
                "code": "model_not_found"
            }
        });
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(403).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
        let client = OpenAiClient::new(KEY, MODEL, &server.uri());
        let result = client
            .generate("a lighthouse at dusk", AssetKind::Hero)
            .await;
        match result {
            Err(OpenAiError::Api { status, message }) => {
                assert_eq!(status, 403);
                assert_eq!(
                    message,
                    "The model `gpt-image-2.6-sunburst` is not enabled for this organization."
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn empty_result_list_gives_no_images() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": []})))
            .expect(1)
            .mount(&server)
            .await;
        let client = OpenAiClient::new(KEY, MODEL, &server.uri());
        // The API can return fewer images than asked for. The client returns
        // what it got, and the app shows a message for an empty list.
        let result = client
            .generate("a lighthouse at dusk", AssetKind::Hero)
            .await
            .unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn bad_base64_is_a_decode_error() {
        let server = MockServer::start().await;
        let body = serde_json::json!({"data": [{"b64_json": "not base64!"}]});
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let client = OpenAiClient::new(KEY, MODEL, &server.uri());
        let result = client
            .generate("a lighthouse at dusk", AssetKind::Hero)
            .await;
        assert!(matches!(result, Err(OpenAiError::Decode(_))), "{result:?}");
    }

    #[tokio::test]
    async fn bad_image_bytes_is_an_image_error() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "data": [{"b64_json": STANDARD.encode("not an image")}]
        });
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let client = OpenAiClient::new(KEY, MODEL, &server.uri());
        let result = client
            .generate("a lighthouse at dusk", AssetKind::Hero)
            .await;
        assert!(matches!(result, Err(OpenAiError::Image(_))), "{result:?}");
    }

    #[tokio::test]
    async fn item_without_b64_json_is_missing_image() {
        let server = MockServer::start().await;
        let body = serde_json::json!({"data": [{"url": "https://example.com/art.png"}]});
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
        let client = OpenAiClient::new(KEY, MODEL, &server.uri());
        let result = client
            .generate("a lighthouse at dusk", AssetKind::Hero)
            .await;
        assert!(
            matches!(result, Err(OpenAiError::MissingImage)),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn slow_response_times_out() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(Duration::from_secs(2))
                    .set_body_json(images_body(&four((1920, 640)))),
            )
            .mount(&server)
            .await;
        let client =
            OpenAiClient::with_timeout(KEY, MODEL, &server.uri(), Duration::from_millis(100));
        let result = client
            .generate("a lighthouse at dusk", AssetKind::Hero)
            .await;
        match result {
            Err(OpenAiError::Request(err)) => assert!(err.is_timeout(), "{err}"),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn key_is_not_in_error_text() {
        const SECRET: &str = "sk-secret-abc";
        let bad_key = OpenAiError::BadKey(401);
        let api = OpenAiError::Api {
            status: 400,
            message: "bad request".to_string(),
        };
        for text in [format!("{bad_key}"), format!("{api}")] {
            assert!(!text.contains(SECRET), "{text}");
        }
        // A real request error must not carry the key either: it sits in a
        // header, not in the URL.
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(2)))
            .mount(&server)
            .await;
        let client =
            OpenAiClient::with_timeout(SECRET, MODEL, &server.uri(), Duration::from_millis(100));
        let err = client
            .generate("a lighthouse at dusk", AssetKind::Hero)
            .await
            .unwrap_err();
        assert!(!format!("{err}").contains(SECRET), "{err}");
        assert!(!format!("{err:?}").contains(SECRET), "{err:?}");
    }

    #[tokio::test]
    async fn trailing_slash_in_the_base_url_is_trimmed() {
        let server = MockServer::start().await;
        let images = four((832, 1248));
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(images_body(&images)))
            .expect(1)
            .mount(&server)
            .await;
        let client = OpenAiClient::new(KEY, MODEL, &format!("{}/", server.uri()));
        let result = client
            .generate("a lighthouse at dusk", AssetKind::Portrait)
            .await
            .unwrap();
        assert_eq!(result.len(), 4);
    }

    #[tokio::test]
    async fn generate_posts_json_and_returns_four_fitted_images() {
        let server = MockServer::start().await;
        let images = four((832, 1248));
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(header("Authorization", "Bearer test-key"))
            .and(body_partial_json(serde_json::json!({
                "model": MODEL,
                "prompt": build_prompt("a lighthouse at dusk", AssetKind::Portrait),
                "n": CANDIDATE_COUNT,
                "size": "832x1248",
                "output_format": "png",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(images_body(&images)))
            .expect(1)
            .mount(&server)
            .await;
        let client = OpenAiClient::new(KEY, MODEL, &server.uri());
        let result = client
            .generate("a lighthouse at dusk", AssetKind::Portrait)
            .await
            .unwrap();
        assert_eq!(result.len(), 4);
        for image in &result {
            assert_eq!((image.width(), image.height()), (600, 900));
        }
    }

    #[tokio::test]
    async fn generate_hero_has_no_background_field() {
        let server = MockServer::start().await;
        let images = four((1920, 640));
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(images_body(&images)))
            .expect(1)
            .mount(&server)
            .await;
        let client = OpenAiClient::new(KEY, MODEL, &server.uri());
        let result = client
            .generate("a lighthouse at dusk", AssetKind::Hero)
            .await
            .unwrap();
        assert_eq!(result.len(), 4);
        for image in &result {
            assert_eq!((image.width(), image.height()), (1920, 620));
        }
        let requests = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert!(body.get("background").is_none(), "{body}");
        assert_eq!(body["size"], "1920x640", "{body}");
    }

    #[tokio::test]
    async fn generate_logo_asks_for_transparent_background() {
        let server = MockServer::start().await;
        // A logo with a transparent border, which `prepare` must trim.
        let logo = DynamicImage::ImageRgba8(RgbaImage::from_fn(1536, 1024, |x, y| {
            if (100..900).contains(&x) && (400..600).contains(&y) {
                Rgba([10, 20, 30, 255])
            } else {
                Rgba([0, 0, 0, 0])
            }
        }));
        let images: Vec<_> = (0..CANDIDATE_COUNT).map(|_| logo.clone()).collect();
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(body_partial_json(serde_json::json!({
                "background": "transparent",
                "size": "1536x1024",
                "output_format": "png",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(images_body(&images)))
            .expect(1)
            .mount(&server)
            .await;
        let client = OpenAiClient::new(KEY, MODEL, &server.uri());
        let result = client
            .generate("a lighthouse at dusk", AssetKind::Logo)
            .await
            .unwrap();
        assert_eq!(result.len(), 4);
        for image in &result {
            assert_eq!((image.width(), image.height()), (800, 200));
            assert!(image.color().has_alpha());
        }
    }
}
