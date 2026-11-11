//! Codex image source: make Steam library art with the Codex CLI
//! (`codex exec`) on the user's ChatGPT plan, without an API key.
//!
//! Codex has no public contract for its `--json` events or for the
//! `generated_images` folder. A Codex update can break this module, so every
//! failure becomes a clear error, and the OpenAI API stays the fallback.

use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use image::{DynamicImage, ImageFormat};
use serde_json::Value;

use crate::assets::AssetKind;
use crate::imaging;
use crate::openai::kind_hint;
use crate::steam::paths::{Os, PathEnv, SystemEnv};

/// One Codex run can take several minutes.
pub const RUN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// `codex --version` answers at once; a hung check must not stay forever.
const VERSION_TIMEOUT: Duration = Duration::from_secs(30);

/// The install steps the settings dialog shows when Codex is not found.
pub const INSTALL_STEPS: [&str; 3] = [
    "Install Node.js (Windows: winget install OpenJS.NodeJS.LTS).",
    "Run npm install -g @openai/codex.",
    "Run codex login and sign in with a ChatGPT account.",
];

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// What one finished `codex` process printed.
#[derive(Debug, Clone, Default)]
pub struct RunOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Runs the `codex` program. Tests use a fake, so no test runs the real
/// Codex.
pub trait CodexRunner: Send + Sync {
    /// The text of `codex --version`, or `None` when Codex is not found.
    fn version(&self) -> BoxFuture<'_, Option<String>>;
    /// Runs `codex` with `args` and a closed stdin. A missing program is an
    /// error of kind `NotFound`.
    fn exec(&self, args: Vec<OsString>) -> BoxFuture<'_, std::io::Result<RunOutput>>;
}

/// Whether the settings dialog found Codex.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexStatus {
    Checking,
    /// The text of `codex --version`.
    Found(String),
    NotFound,
}

/// Asks `runner` for the Codex version.
pub async fn check(runner: &dyn CodexRunner) -> CodexStatus {
    match runner.version().await {
        Some(version) => CodexStatus::Found(version),
        None => CodexStatus::NotFound,
    }
}

/// The file names of the Codex program, in the order to try them. npm
/// installs `codex.cmd` on Windows.
pub fn program_names(os: Os) -> &'static [&'static str] {
    match os {
        Os::Windows => &["codex.cmd", "codex.exe"],
        Os::Linux | Os::MacOs => &["codex"],
    }
}

/// The first Codex program in the folders of `path_var` (the `PATH`
/// value).
pub fn find_program(os: Os, path_var: Option<&OsStr>) -> Option<PathBuf> {
    std::env::split_paths(path_var?).find_map(|dir| {
        program_names(os)
            .iter()
            .map(|name| dir.join(name))
            .find(|path| path.is_file())
    })
}

/// The facts the app reads from the `--json` event lines of one run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Events {
    /// From the `thread.started` event.
    pub thread_id: Option<String>,
    /// The text of each `agent_message` item, in order.
    pub messages: Vec<String>,
    /// The messages of `error` and `turn.failed` events, without repeats.
    pub errors: Vec<String>,
}

/// Reads the event lines. A line that is not a JSON object is skipped.
pub fn parse_events(stdout: &str) -> Events {
    let mut events = Events::default();
    for line in stdout.lines() {
        let Ok(Value::Object(event)) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_string);
        let error = match event.get("type").and_then(Value::as_str) {
            Some("thread.started") => {
                if events.thread_id.is_none() {
                    events.thread_id = text(event.get("thread_id"));
                }
                None
            }
            Some("item.completed") => {
                let item = event.get("item");
                if item
                    .and_then(|item| item.get("type"))
                    .and_then(Value::as_str)
                    == Some("agent_message")
                    && let Some(message) = text(item.and_then(|item| item.get("text")))
                {
                    events.messages.push(message);
                }
                None
            }
            Some("error") => text(event.get("message")),
            Some("turn.failed") => text(event.get("error").and_then(|error| error.get("message"))),
            _ => None,
        };
        if let Some(error) = error
            && !events.errors.contains(&error)
        {
            events.errors.push(error);
        }
    }
    events
}

/// The shape Codex must make for `kind`.
fn shape_hint(kind: AssetKind) -> &'static str {
    match kind {
        AssetKind::Portrait => "Make it a vertical image in a 2:3 ratio.",
        AssetKind::Wide => "Make it a wide image in a 920:430 ratio (about 2.14:1).",
        AssetKind::Hero => "Make it as wide as allowed, in a 3:1 ratio.",
        AssetKind::Logo => "Make it a wide image with a fully transparent background.",
    }
}

/// The instruction for one run. It is one line, so Windows can pass it to
/// `codex.cmd` safely.
pub fn instruction(prompt: &str, kind: AssetKind, modify: bool) -> String {
    let mut parts = vec![
        "Generate exactly one image with your image generation tool: Steam library art for a game.",
        shape_hint(kind),
        kind_hint(kind),
    ];
    if modify {
        parts.push("Edit the attached image as the request says.");
    }
    parts.push(
        "Do not write or change any other file. Reply with only the path of the saved image.",
    );
    let mut text = parts.join(" ");
    let request = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
    if !request.is_empty() {
        text.push_str(" Request: ");
        text.push_str(&request);
    }
    text
}

/// The arguments of `codex` for one run. `source` is the image to modify.
pub fn exec_args(work_dir: &Path, source: Option<&Path>, instruction: &str) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "exec",
        "--json",
        "--skip-git-repo-check",
        "--ephemeral",
        "-s",
        "read-only",
        "-C",
    ]
    .into_iter()
    .map(OsString::from)
    .collect();
    args.push(work_dir.into());
    if let Some(source) = source {
        args.push("-i".into());
        args.push(source.into());
    }
    // `-i` takes every value after it, so `--` ends the list before the
    // instruction.
    args.push("--".into());
    args.push(instruction.into());
    args
}

/// The PNG one run saved, and `true` when it is in Codex's own folder for
/// this run (the app then moves it out).
pub fn saved_image(events: &Events, codex_home: Option<&Path>) -> Option<(PathBuf, bool)> {
    if let (Some(home), Some(thread)) = (codex_home, &events.thread_id) {
        let folder = home.join("generated_images").join(thread);
        let newest = std::fs::read_dir(&folder)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| is_png(path) && path.is_file())
            .max_by_key(|path| {
                std::fs::metadata(path)
                    .and_then(|meta| meta.modified())
                    .ok()
            });
        if let Some(path) = newest {
            return Some((path, true));
        }
    }
    let message = events.messages.last()?;
    let path = PathBuf::from(message.trim().trim_matches(['`', '"', '\'']));
    (is_png(&path) && path.is_file()).then_some((path, false))
}

fn is_png(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"))
}

/// The reason a run made no image: Codex's errors, else its last message,
/// else the end of stderr.
fn no_image_reason(events: &Events, output: &RunOutput) -> String {
    if !events.errors.is_empty() {
        return events.errors.join("; ");
    }
    if let Some(message) = events.messages.last() {
        return message.trim().to_string();
    }
    let stderr = output.stderr.trim();
    let tail: String = {
        let chars: Vec<char> = stderr.chars().collect();
        chars[chars.len().saturating_sub(500)..].iter().collect()
    };
    if tail.is_empty() {
        "Codex printed nothing".to_string()
    } else {
        tail
    }
}

/// One Codex run: start it, find its image, move the image out of the
/// Codex folder and fit it to `kind`.
async fn run_one(
    runner: Arc<dyn CodexRunner>,
    codex_home: Option<PathBuf>,
    timeout: Duration,
    args: Vec<OsString>,
    kind: AssetKind,
) -> Result<DynamicImage, CodexError> {
    let output = match tokio::time::timeout(timeout, runner.exec(args)).await {
        Err(_) => return Err(CodexError::Timeout(timeout.as_secs() / 60)),
        Ok(Err(err)) if err.kind() == std::io::ErrorKind::NotFound => {
            return Err(CodexError::NotInstalled);
        }
        Ok(Err(err)) => return Err(CodexError::Run(err)),
        Ok(Ok(output)) => output,
    };
    let events = parse_events(&output.stdout);
    let Some((path, generated)) = saved_image(&events, codex_home.as_deref()) else {
        return Err(CodexError::NoImage(no_image_reason(&events, &output)));
    };
    let bytes = tokio::fs::read(&path)
        .await
        .map_err(|source| CodexError::Read {
            path: path.clone(),
            source,
        })?;
    // A file that Codex only names can be someone else's file, so it stays.
    if generated {
        let _ = tokio::fs::remove_file(&path).await;
        if let Some(folder) = path.parent() {
            let _ = tokio::fs::remove_dir(folder).await;
        }
    }
    let image = image::load_from_memory(&bytes)?;
    Ok(imaging::prepare(&image, kind))
}

#[derive(Debug, thiserror::Error)]
pub enum CodexError {
    #[error("Codex is not installed. Open Settings for the install steps.")]
    NotInstalled,
    #[error("cannot run Codex: {0}")]
    Run(std::io::Error),
    #[error("Codex made no image in {0} minutes")]
    Timeout(u64),
    #[error("Codex made no image: {0}")]
    NoImage(String),
    #[error("cannot read the Codex image {}: {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("the Codex image is not valid: {0}")]
    Image(#[from] image::ImageError),
    #[error("cannot prepare the Codex work folder: {0}")]
    WorkDir(std::io::Error),
}

/// Makes art with one Codex run per image, all at the same time.
#[derive(Clone)]
pub struct CodexClient {
    runner: Arc<dyn CodexRunner>,
    codex_home: Option<PathBuf>,
    timeout: Duration,
}

impl CodexClient {
    /// `codex_home` is [`crate::steam::paths::codex_home`] outside tests.
    pub fn new(runner: Arc<dyn CodexRunner>, codex_home: Option<PathBuf>) -> Self {
        Self::with_timeout(runner, codex_home, RUN_TIMEOUT)
    }

    /// Like [`CodexClient::new`], with a run timeout of your choice.
    pub fn with_timeout(
        runner: Arc<dyn CodexRunner>,
        codex_home: Option<PathBuf>,
        timeout: Duration,
    ) -> Self {
        Self {
            runner,
            codex_home,
            timeout,
        }
    }

    /// Makes `count` new images from `prompt`, ready to write as `kind`.
    pub async fn generate(
        &self,
        prompt: &str,
        kind: AssetKind,
        count: u8,
    ) -> Result<Vec<DynamicImage>, CodexError> {
        self.run_all(None, prompt, kind, count).await
    }

    /// Edits `source` with `prompt` and returns `count` images ready to
    /// write as `kind`.
    pub async fn modify(
        &self,
        source: &DynamicImage,
        prompt: &str,
        kind: AssetKind,
        count: u8,
    ) -> Result<Vec<DynamicImage>, CodexError> {
        self.run_all(Some(source), prompt, kind, count).await
    }

    /// Starts `count` runs at the same time in one temp work folder. One
    /// failed run does not hide the images of the others.
    async fn run_all(
        &self,
        source: Option<&DynamicImage>,
        prompt: &str,
        kind: AssetKind,
        count: u8,
    ) -> Result<Vec<DynamicImage>, CodexError> {
        let work = tempfile::Builder::new()
            .prefix("steam-asset-manager-codex-")
            .tempdir()
            .map_err(CodexError::WorkDir)?;
        let source_path = match source {
            Some(image) => {
                let path = work.path().join("source.png");
                image.save_with_format(&path, ImageFormat::Png)?;
                Some(path)
            }
            None => None,
        };
        let text = instruction(prompt, kind, source.is_some());
        let args = exec_args(work.path(), source_path.as_deref(), &text);

        let mut runs = tokio::task::JoinSet::new();
        for _ in 0..count {
            runs.spawn(run_one(
                self.runner.clone(),
                self.codex_home.clone(),
                self.timeout,
                args.clone(),
                kind,
            ));
        }
        let mut images = Vec::new();
        let mut first_error = None;
        while let Some(joined) = runs.join_next().await {
            match joined {
                Ok(Ok(image)) => images.push(image),
                Ok(Err(err)) => {
                    first_error.get_or_insert(err);
                }
                Err(err) => {
                    first_error.get_or_insert(CodexError::Run(std::io::Error::other(err)));
                }
            }
        }
        // The work folder must outlive every run.
        drop(work);
        match first_error {
            Some(err) if images.is_empty() => Err(err),
            _ => Ok(images),
        }
    }
}

/// The real Codex program on the `PATH`.
pub struct SystemCodex;

impl CodexRunner for SystemCodex {
    fn version(&self) -> BoxFuture<'_, Option<String>> {
        Box::pin(async {
            let output = tokio::time::timeout(
                VERSION_TIMEOUT,
                run_system(vec![OsString::from("--version")]),
            )
            .await
            .ok()?
            .ok()?;
            let version = output.stdout.trim();
            (output.success && !version.is_empty()).then(|| version.to_string())
        })
    }

    fn exec(&self, args: Vec<OsString>) -> BoxFuture<'_, std::io::Result<RunOutput>> {
        Box::pin(run_system(args))
    }
}

/// Runs the Codex program of the `PATH` with a closed stdin. Without it,
/// Codex waits for more input.
async fn run_system(args: Vec<OsString>) -> std::io::Result<RunOutput> {
    let program = find_program(SystemEnv.os(), std::env::var_os("PATH").as_deref())
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        // Without it, each run opens a console window.
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().await?;
    Ok(RunOutput {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use image::{Rgba, RgbaImage};
    use tempfile::TempDir;

    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        let image =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(width, height, Rgba([7, 8, 9, 255])));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    fn line(value: Value) -> String {
        value.to_string()
    }

    fn thread_started(id: &str) -> String {
        line(serde_json::json!({"type": "thread.started", "thread_id": id}))
    }

    fn agent_message(text: &str) -> String {
        line(serde_json::json!({
            "type": "item.completed",
            "item": {"id": "item_1", "type": "agent_message", "text": text}
        }))
    }

    /// What the fake does on one run.
    #[derive(Clone, Copy)]
    enum Reply {
        /// Saves a PNG in the Codex folder of a new thread.
        Image(u32, u32),
        /// Reports a failed turn.
        Fail,
        /// The program is missing.
        Missing,
        /// Never ends.
        Hang,
    }

    struct FakeRunner {
        home: PathBuf,
        replies: Vec<Reply>,
        runs: AtomicUsize,
        calls: Mutex<Vec<Vec<OsString>>>,
    }

    impl FakeRunner {
        fn new(home: &Path, replies: Vec<Reply>) -> Arc<Self> {
            Arc::new(FakeRunner {
                home: home.to_path_buf(),
                replies,
                runs: AtomicUsize::new(0),
                calls: Mutex::new(Vec::new()),
            })
        }
    }

    impl CodexRunner for FakeRunner {
        fn version(&self) -> BoxFuture<'_, Option<String>> {
            Box::pin(async { Some("codex-cli 0.167.0".to_string()) })
        }

        fn exec(&self, args: Vec<OsString>) -> BoxFuture<'_, std::io::Result<RunOutput>> {
            self.calls.lock().unwrap().push(args);
            let run = self.runs.fetch_add(1, Ordering::SeqCst);
            let reply = self.replies[run % self.replies.len()];
            let home = self.home.clone();
            Box::pin(async move {
                let thread = format!("thread-{run}");
                match reply {
                    Reply::Image(width, height) => {
                        let folder = home.join("generated_images").join(&thread);
                        fs::create_dir_all(&folder).unwrap();
                        let file = folder.join("ig_1.png");
                        fs::write(&file, png_bytes(width, height)).unwrap();
                        Ok(RunOutput {
                            success: true,
                            stdout: [thread_started(&thread), agent_message("done")].join("\n"),
                            stderr: String::new(),
                        })
                    }
                    Reply::Fail => Ok(RunOutput {
                        success: false,
                        stdout: [
                            thread_started(&thread),
                            line(serde_json::json!({
                                "type": "turn.failed",
                                "error": {"message": "usage limit reached"}
                            })),
                        ]
                        .join("\n"),
                        stderr: String::new(),
                    }),
                    Reply::Missing => Err(std::io::Error::from(std::io::ErrorKind::NotFound)),
                    Reply::Hang => {
                        std::future::pending::<()>().await;
                        unreachable!()
                    }
                }
            })
        }
    }

    struct NoCodex;

    impl CodexRunner for NoCodex {
        fn version(&self) -> BoxFuture<'_, Option<String>> {
            Box::pin(async { None })
        }
        fn exec(&self, _args: Vec<OsString>) -> BoxFuture<'_, std::io::Result<RunOutput>> {
            Box::pin(async { Err(std::io::Error::from(std::io::ErrorKind::NotFound)) })
        }
    }

    #[tokio::test]
    async fn check_reports_the_version() {
        let home = TempDir::new().unwrap();
        let runner = FakeRunner::new(home.path(), vec![Reply::Fail]);
        assert_eq!(
            check(runner.as_ref()).await,
            CodexStatus::Found("codex-cli 0.167.0".to_string())
        );
    }

    #[tokio::test]
    async fn check_reports_a_missing_codex() {
        assert_eq!(check(&NoCodex).await, CodexStatus::NotFound);
    }

    #[test]
    fn windows_looks_for_the_npm_cmd_file() {
        assert_eq!(program_names(Os::Windows), &["codex.cmd", "codex.exe"]);
        assert_eq!(program_names(Os::Linux), &["codex"]);
        assert_eq!(program_names(Os::MacOs), &["codex"]);
    }

    #[test]
    fn find_program_takes_the_first_folder_with_codex() {
        let temp = TempDir::new().unwrap();
        let empty = temp.path().join("empty");
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        for dir in [&empty, &first, &second] {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(first.join("codex"), "").unwrap();
        fs::write(second.join("codex"), "").unwrap();
        let path_var = std::env::join_paths([&empty, &first, &second]).unwrap();
        assert_eq!(
            find_program(Os::Linux, Some(&path_var)),
            Some(first.join("codex"))
        );
    }

    #[test]
    fn find_program_on_windows_finds_codex_cmd() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("codex"), "").unwrap();
        fs::write(temp.path().join("codex.cmd"), "").unwrap();
        let path_var = std::env::join_paths([temp.path()]).unwrap();
        assert_eq!(
            find_program(Os::Windows, Some(&path_var)),
            Some(temp.path().join("codex.cmd"))
        );
    }

    #[test]
    fn find_program_ignores_a_folder_named_codex() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join("codex")).unwrap();
        let path_var = std::env::join_paths([temp.path()]).unwrap();
        assert_eq!(find_program(Os::Linux, Some(&path_var)), None);
        assert_eq!(find_program(Os::Linux, None), None);
    }

    #[test]
    fn events_give_the_thread_messages_and_errors() {
        let stdout = [
            thread_started("019a-thread"),
            "not json".to_string(),
            line(serde_json::json!({"type": "turn.started"})),
            agent_message("first"),
            line(serde_json::json!({"type": "error", "message": "stream lost"})),
            line(serde_json::json!({"type": "error", "message": "stream lost"})),
            line(serde_json::json!({"type": "turn.failed", "error": {"message": "limit"}})),
            agent_message("/tmp/x.png"),
        ]
        .join("\n");
        assert_eq!(
            parse_events(&stdout),
            Events {
                thread_id: Some("019a-thread".to_string()),
                messages: vec!["first".to_string(), "/tmp/x.png".to_string()],
                errors: vec!["stream lost".to_string(), "limit".to_string()],
            }
        );
    }

    #[test]
    fn empty_output_gives_no_events() {
        assert_eq!(parse_events(""), Events::default());
    }

    #[test]
    fn instruction_asks_for_one_image_and_the_path() {
        let text = instruction("A neon city", AssetKind::Portrait, false);
        assert!(text.starts_with("Generate exactly one image"), "{text}");
        assert!(text.contains("image generation tool"), "{text}");
        assert!(text.contains("2:3"), "{text}");
        assert!(
            text.contains("Reply with only the path of the saved image."),
            "{text}"
        );
        assert!(text.ends_with("Request: A neon city"), "{text}");
        assert!(!text.contains("attached"), "{text}");
    }

    #[test]
    fn instruction_names_the_shape_of_each_kind() {
        assert!(instruction("x", AssetKind::Wide, false).contains("920:430"));
        let hero = instruction("x", AssetKind::Hero, false);
        assert!(hero.contains("3:1"), "{hero}");
        assert!(hero.contains("no text"), "{hero}");
        let logo = instruction("x", AssetKind::Logo, false);
        assert!(logo.contains("transparent background"), "{logo}");
    }

    #[test]
    fn instruction_for_modify_edits_the_attached_image() {
        let text = instruction("Make it red", AssetKind::Hero, true);
        assert!(text.contains("Edit the attached image"), "{text}");
    }

    #[test]
    fn instruction_is_one_line() {
        let text = instruction("  line one\n\nline two \r\n", AssetKind::Logo, false);
        assert!(!text.contains('\n') && !text.contains('\r'), "{text}");
        assert!(text.ends_with("Request: line one line two"), "{text}");
    }

    #[test]
    fn blank_prompt_gives_no_request_part() {
        let text = instruction("  ", AssetKind::Logo, false);
        assert!(!text.contains("Request:"), "{text}");
    }

    #[test]
    fn exec_args_for_generate() {
        let args = exec_args(Path::new("/work"), None, "make art");
        let expected: Vec<OsString> = [
            "exec",
            "--json",
            "--skip-git-repo-check",
            "--ephemeral",
            "-s",
            "read-only",
            "-C",
            "/work",
            "--",
            "make art",
        ]
        .into_iter()
        .map(OsString::from)
        .collect();
        assert_eq!(args, expected);
    }

    #[test]
    fn exec_args_for_modify_end_the_image_list() {
        let args = exec_args(
            Path::new("/work"),
            Some(Path::new("/work/source.png")),
            "edit",
        );
        let tail: Vec<&OsStr> = args
            .iter()
            .rev()
            .take(4)
            .rev()
            .map(|a| a.as_os_str())
            .collect();
        assert_eq!(
            tail,
            ["-i", "/work/source.png", "--", "edit"].map(OsStr::new)
        );
    }

    #[test]
    fn saved_image_takes_the_newest_png_of_the_thread() {
        let home = TempDir::new().unwrap();
        let folder = home.path().join("generated_images/t1");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("old.png"), png_bytes(2, 2)).unwrap();
        fs::write(folder.join("notes.txt"), "x").unwrap();
        let old = fs::File::options()
            .write(true)
            .open(folder.join("old.png"))
            .unwrap();
        old.set_modified(std::time::SystemTime::UNIX_EPOCH).unwrap();
        fs::write(folder.join("new.png"), png_bytes(2, 2)).unwrap();
        let events = Events {
            thread_id: Some("t1".to_string()),
            ..Events::default()
        };
        assert_eq!(
            saved_image(&events, Some(home.path())),
            Some((folder.join("new.png"), true))
        );
    }

    #[test]
    fn saved_image_falls_back_to_the_path_in_the_last_message() {
        let home = TempDir::new().unwrap();
        let file = home.path().join("elsewhere.png");
        fs::write(&file, png_bytes(2, 2)).unwrap();
        let events = Events {
            thread_id: Some("no-folder".to_string()),
            messages: vec!["working".to_string(), format!("`{}`\n", file.display())],
            errors: Vec::new(),
        };
        assert_eq!(saved_image(&events, Some(home.path())), Some((file, false)));
    }

    #[test]
    fn saved_image_ignores_a_message_that_is_no_png_file() {
        let home = TempDir::new().unwrap();
        let events = Events {
            thread_id: None,
            messages: vec![home.path().join("missing.png").display().to_string()],
            errors: Vec::new(),
        };
        assert_eq!(saved_image(&events, Some(home.path())), None);
        let events = Events {
            messages: vec!["I cannot make images".to_string()],
            ..Events::default()
        };
        assert_eq!(saved_image(&events, None), None);
    }

    #[tokio::test]
    async fn generate_makes_four_prepared_images() {
        let home = TempDir::new().unwrap();
        let runner = FakeRunner::new(home.path(), vec![Reply::Image(2172, 724)]);
        let client = CodexClient::new(runner.clone(), Some(home.path().to_path_buf()));
        let images = client
            .generate("A castle", AssetKind::Hero, 4)
            .await
            .unwrap();
        assert_eq!(images.len(), 4);
        for image in &images {
            assert_eq!((image.width(), image.height()), (1920, 620));
        }
        let calls = runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 4);
        assert!(!calls[0].contains(&OsString::from("-i")));
        // Each PNG moved out of the Codex folder.
        let left: Vec<_> = walk_pngs(&home.path().join("generated_images"));
        assert!(left.is_empty(), "{left:?}");
    }

    #[tokio::test]
    async fn the_run_count_equals_the_requested_count() {
        for count in 1..=4u8 {
            let home = TempDir::new().unwrap();
            let runner = FakeRunner::new(home.path(), vec![Reply::Image(2172, 724)]);
            let client = CodexClient::new(runner.clone(), Some(home.path().to_path_buf()));
            let images = client
                .generate("A castle", AssetKind::Hero, count)
                .await
                .unwrap();
            assert_eq!(images.len(), usize::from(count));
            assert_eq!(runner.calls.lock().unwrap().len(), usize::from(count));
        }
    }

    #[tokio::test]
    async fn modify_starts_the_requested_number_of_runs() {
        let home = TempDir::new().unwrap();
        let runner = FakeRunner::new(home.path(), vec![Reply::Image(900, 1350)]);
        let client = CodexClient::new(runner.clone(), Some(home.path().to_path_buf()));
        let source = DynamicImage::ImageRgba8(RgbaImage::new(600, 900));
        let images = client
            .modify(&source, "Make it red", AssetKind::Portrait, 2)
            .await
            .unwrap();
        assert_eq!(images.len(), 2);
        assert_eq!(runner.calls.lock().unwrap().len(), 2);
    }

    fn walk_pngs(dir: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    found.extend(walk_pngs(&path));
                } else if path.extension().is_some_and(|ext| ext == "png") {
                    found.push(path);
                }
            }
        }
        found
    }

    #[tokio::test]
    async fn modify_attaches_the_source_image() {
        let home = TempDir::new().unwrap();
        let runner = FakeRunner::new(home.path(), vec![Reply::Image(900, 1350)]);
        let client = CodexClient::new(runner.clone(), Some(home.path().to_path_buf()));
        let source = DynamicImage::ImageRgba8(RgbaImage::new(600, 900));
        let images = client
            .modify(&source, "Make it red", AssetKind::Portrait, 4)
            .await
            .unwrap();
        assert_eq!(images.len(), 4);
        assert_eq!((images[0].width(), images[0].height()), (600, 900));
        let calls = runner.calls.lock().unwrap();
        let args = &calls[0];
        let at = args.iter().position(|arg| arg == "-i").expect("an -i flag");
        let attached = PathBuf::from(&args[at + 1]);
        assert_eq!(attached.extension().unwrap(), "png");
        assert_eq!(args[at + 2], "--");
        // The work folder goes when the call ends.
        assert!(!attached.exists());
    }

    #[tokio::test]
    async fn one_failed_run_keeps_the_other_images() {
        let home = TempDir::new().unwrap();
        let runner = FakeRunner::new(home.path(), vec![Reply::Fail, Reply::Image(1472, 688)]);
        let client = CodexClient::new(runner, Some(home.path().to_path_buf()));
        let images = client.generate("x", AssetKind::Wide, 4).await.unwrap();
        assert_eq!(images.len(), 2);
    }

    #[tokio::test]
    async fn all_failed_runs_report_the_codex_error() {
        let home = TempDir::new().unwrap();
        let runner = FakeRunner::new(home.path(), vec![Reply::Fail]);
        let client = CodexClient::new(runner, Some(home.path().to_path_buf()));
        let err = client.generate("x", AssetKind::Wide, 4).await.unwrap_err();
        assert!(matches!(err, CodexError::NoImage(_)), "{err}");
        assert!(err.to_string().contains("usage limit reached"), "{err}");
    }

    #[tokio::test]
    async fn a_missing_program_says_codex_is_not_installed() {
        let home = TempDir::new().unwrap();
        let runner = FakeRunner::new(home.path(), vec![Reply::Missing]);
        let client = CodexClient::new(runner, Some(home.path().to_path_buf()));
        let err = client.generate("x", AssetKind::Logo, 4).await.unwrap_err();
        assert!(matches!(err, CodexError::NotInstalled), "{err}");
    }

    #[tokio::test]
    async fn a_hung_run_times_out() {
        let home = TempDir::new().unwrap();
        let runner = FakeRunner::new(home.path(), vec![Reply::Hang]);
        let client = CodexClient::with_timeout(
            runner,
            Some(home.path().to_path_buf()),
            Duration::from_millis(50),
        );
        let err = client.generate("x", AssetKind::Logo, 4).await.unwrap_err();
        assert!(matches!(err, CodexError::Timeout(_)), "{err}");
    }
}
