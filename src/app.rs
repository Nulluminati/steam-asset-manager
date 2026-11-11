//! The iced application: state, messages and the update loop.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use iced::widget::image::Handle;
use iced::widget::text_editor;
use iced::{Element, Task};
use image::DynamicImage;

use crate::assets::AssetKind;
use crate::codex::{CodexClient, CodexRunner, CodexStatus, SystemCodex};
use crate::config::{self, Config, ImageSource};
use crate::openai::OpenAiClient;
use crate::secrets::{Secrets, Service};
use crate::sgdb::{Candidate, SGDB_PAGE_SIZE, SgdbClient};
use crate::steam::defaults::{self, DefaultArt};
use crate::steam::library::{LibraryEntry, sort_entries};
use crate::steam::paths::{self, PathEnv, SystemEnv};
use crate::steam::users::SteamUser;
use crate::steam::{grid, owned, shortcuts, users};
use crate::ui::detail::SlotArt;
use crate::ui::settings::{SettingsField, SettingsForm};

/// The tab of the right panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelTab {
    SteamGridDb,
    Generate,
    Modify,
}

impl PanelTab {
    pub const ALL: [PanelTab; 3] = [PanelTab::SteamGridDb, PanelTab::Generate, PanelTab::Modify];

    /// The name the segmented control shows for this tab.
    pub fn label(self) -> &'static str {
        match self {
            PanelTab::SteamGridDb => "SteamGridDB",
            PanelTab::Generate => "Generate",
            PanelTab::Modify => "Modify",
        }
    }
}

/// The number of cells of the candidate grid: one SteamGridDB page.
pub const GRID_CELLS: usize = 4;

/// The thumbnail of one SteamGridDB candidate.
#[derive(Debug, Clone, Copy)]
pub enum Thumb<'a> {
    Loading,
    Ready(&'a Handle),
    Failed,
}

/// The images a SteamGridDB search found, shown one page of
/// [`GRID_CELLS`] at a time.
#[derive(Debug, Clone)]
pub struct SgdbResults {
    /// The entry and the slot the search was made for.
    appid: u32,
    kind: AssetKind,
    found: Vec<Candidate>,
    /// The page the grid shows, from 0.
    page: usize,
    /// The thumbnails by candidate ID; `None` when the download failed.
    thumbs: HashMap<u64, Option<Handle>>,
    /// The API page the last list request asked for.
    api_page: u32,
    /// The API can hold more images than `found`.
    more: bool,
}

impl SgdbResults {
    fn new(appid: u32, kind: AssetKind, found: Vec<Candidate>) -> Self {
        SgdbResults {
            appid,
            kind,
            more: found.len() >= SGDB_PAGE_SIZE,
            found,
            page: 0,
            thumbs: HashMap::new(),
            api_page: 0,
        }
    }

    /// Every candidate the app has loaded.
    pub fn candidates(&self) -> &[Candidate] {
        &self.found
    }

    /// The index in [`Self::candidates`] of the first cell of the page.
    pub fn first_index(&self) -> usize {
        self.page * GRID_CELLS
    }

    /// The candidates of the page the grid shows.
    pub fn page_candidates(&self) -> &[Candidate] {
        let start = self.first_index().min(self.found.len());
        let end = (start + GRID_CELLS).min(self.found.len());
        &self.found[start..end]
    }

    pub fn has_previous(&self) -> bool {
        self.page > 0
    }

    pub fn has_next(&self) -> bool {
        self.first_index() + GRID_CELLS < self.found.len() || self.more
    }

    /// The text between the page buttons, for example `5 to 8 of 31`.
    pub fn counter(&self) -> String {
        crate::ui::panel::page_counter(
            self.first_index(),
            self.page_candidates().len(),
            self.found.len(),
            self.more,
        )
    }

    pub fn thumb(&self, id: u64) -> Thumb<'_> {
        match self.thumbs.get(&id) {
            None => Thumb::Loading,
            Some(Some(handle)) => Thumb::Ready(handle),
            Some(None) => Thumb::Failed,
        }
    }

    /// The candidates of the page that have no thumbnail yet.
    pub fn thumbs_to_load(&self) -> Vec<Candidate> {
        self.page_candidates()
            .iter()
            .filter(|candidate| !self.thumbs.contains_key(&candidate.id))
            .cloned()
            .collect()
    }

    /// The next page needs images the app has not loaded yet.
    fn next_needs_fetch(&self) -> bool {
        self.more && self.first_index() + 2 * GRID_CELLS > self.found.len()
    }

    /// Adds one more API page. A page without new IDs ends the list, so an
    /// API that ignores the page number cannot repeat the same images.
    fn append(&mut self, api_page: u32, found: Vec<Candidate>) {
        let full = found.len() >= SGDB_PAGE_SIZE;
        let before = self.found.len();
        for candidate in found {
            if !self.found.iter().any(|known| known.id == candidate.id) {
                self.found.push(candidate);
            }
        }
        self.api_page = api_page;
        self.more = full && self.found.len() > before;
    }
}

/// The one line the app shows under the panel: a note or a failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Info(String),
    Error(String),
}

impl Status {
    pub fn text(&self) -> &str {
        match self {
            Status::Info(text) | Status::Error(text) => text,
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Status::Error(_))
    }
}

/// The parts of the environment that `update` reads: keys, the Steam
/// environment, HTTP base URLs, the config file and the Codex program.
pub struct Services {
    pub secrets: Secrets,
    pub env: Arc<dyn PathEnv + Send + Sync>,
    pub config_path: Option<PathBuf>,
    pub steam_api_base: String,
    pub sgdb_base: String,
    pub openai_base: String,
    pub steam_cdn_base: String,
    pub codex: Arc<dyn CodexRunner>,
    /// The Codex home folder, where Codex saves its images.
    pub codex_home: Option<PathBuf>,
}

impl Default for Services {
    fn default() -> Self {
        Services {
            secrets: Secrets::system(),
            env: Arc::new(SystemEnv),
            config_path: config::default_path(),
            steam_api_base: crate::steam::owned::STEAM_API_BASE.to_string(),
            sgdb_base: crate::sgdb::SGDB_BASE.to_string(),
            openai_base: crate::openai::OPENAI_BASE.to_string(),
            steam_cdn_base: crate::steam::defaults::STEAM_CDN_BASE.to_string(),
            codex: Arc::new(SystemCodex),
            codex_home: paths::codex_home(std::env::var_os("CODEX_HOME"), SystemEnv.home_dir()),
        }
    }
}

/// The library data the start-up task returns.
#[derive(Debug, Clone)]
pub struct LoadedLibrary {
    pub config: Config,
    pub steam_root: Option<PathBuf>,
    pub account_id: Option<u32>,
    pub entries: Vec<LibraryEntry>,
    pub status: Option<Status>,
}

/// What the start-up task reads.
pub struct StartupInput {
    pub env: Arc<dyn PathEnv + Send + Sync>,
    pub config_path: Option<PathBuf>,
    pub steam_api_key: Option<String>,
    pub steam_api_base: String,
}

#[derive(Debug, Clone)]
pub enum Message {
    /// The result of the start-up task.
    Loaded(Box<LoadedLibrary>),
    /// The text of the library search box changed.
    FilterChanged(String),
    /// The user picked a library entry.
    EntrySelected(u32),
    /// The user picked a panel tab.
    TabSelected(PanelTab),
    /// The prompt text changed.
    PromptChanged(String),
    /// The primary action button was pressed.
    ActionPressed,
    /// A request for candidates finished. The appid and the kind say which
    /// slot the images were made for, so a late reply cannot land in the
    /// wrong slot.
    CandidatesArrived {
        appid: u32,
        kind: AssetKind,
        result: Result<Vec<DynamicImage>, String>,
    },
    /// The user applied the candidate at this index. On SteamGridDB the
    /// index counts across all pages.
    ApplyPressed(usize),
    /// A SteamGridDB list request for one API page finished.
    SgdbListed {
        appid: u32,
        kind: AssetKind,
        api_page: u32,
        result: Result<Vec<Candidate>, String>,
    },
    /// Thumbnails of one SteamGridDB page by candidate ID; `None` when the
    /// download failed.
    SgdbThumbsLoaded {
        appid: u32,
        kind: AssetKind,
        thumbs: Vec<(u64, Option<Handle>)>,
    },
    SgdbPrevious,
    SgdbNext,
    /// The full image of the applied SteamGridDB candidate, prepared.
    SgdbDownloaded {
        appid: u32,
        kind: AssetKind,
        result: Result<DynamicImage, String>,
    },
    /// The Target dropdown: it sets the target and keeps the tab.
    TargetSelected(AssetKind),
    /// An edit in the multi-line prompt box.
    PromptEdited(text_editor::Action),
    /// The current art of the slots of one entry finished loading.
    SlotArtLoaded {
        appid: u32,
        art: Vec<(AssetKind, SlotArt)>,
    },
    SettingsOpened,
    SettingsClosed,
    SettingsChanged(SettingsField, String),
    /// The image source option of the settings dialog.
    ImageSourceSelected(ImageSource),
    /// The images per request option of the settings dialog.
    ImagesPerRequestSelected(u8),
    /// The settings dialog finished looking for Codex.
    CodexChecked(CodexStatus),
    SettingsSaved,
}

pub struct State {
    services: Services,
    config: Config,
    steam_root: Option<PathBuf>,
    account_id: Option<u32>,
    entries: Vec<LibraryEntry>,
    filter: String,
    selected: Option<u32>,
    target: AssetKind,
    tab: PanelTab,
    prompt: String,
    candidates: Vec<DynamicImage>,
    /// The images the view shows for `candidates`, in the same order.
    candidate_handles: Vec<Handle>,
    /// The entry and the slot the candidates were made for.
    candidate_context: Option<(u32, AssetKind)>,
    /// The SteamGridDB search results. Only one of these and `candidates`
    /// holds images at a time.
    sgdb: Option<SgdbResults>,
    busy: bool,
    status: Option<Status>,
    /// The current art of each slot of the selected entry.
    slot_art: HashMap<AssetKind, SlotArt>,
    /// The text of the prompt box; `prompt` holds the same text.
    prompt_editor: text_editor::Content,
    /// The open settings dialog.
    settings: Option<SettingsForm>,
}

impl State {
    pub fn new(services: Services) -> Self {
        State {
            services,
            config: Config::default(),
            steam_root: None,
            account_id: None,
            entries: Vec::new(),
            filter: String::new(),
            selected: None,
            target: AssetKind::Portrait,
            tab: PanelTab::SteamGridDb,
            prompt: String::new(),
            candidates: Vec::new(),
            candidate_handles: Vec::new(),
            candidate_context: None,
            sgdb: None,
            busy: false,
            status: None,
            slot_art: HashMap::new(),
            prompt_editor: text_editor::Content::new(),
            settings: None,
        }
    }

    /// The state at start-up, with the task that loads the library.
    pub fn boot() -> (State, Task<Message>) {
        let mut state = State::new(Services::default());
        let task = state.start();
        (state, task)
    }

    pub fn entries(&self) -> &[LibraryEntry] {
        &self.entries
    }

    /// The entries the search box lets through, in library order.
    pub fn visible_entries(&self) -> Vec<&LibraryEntry> {
        let filter = self.filter.trim().to_lowercase();
        self.entries
            .iter()
            .filter(|entry| filter.is_empty() || entry.name.to_lowercase().contains(&filter))
            .collect()
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn selected_appid(&self) -> Option<u32> {
        self.selected
    }

    pub fn selected_entry(&self) -> Option<&LibraryEntry> {
        let appid = self.selected?;
        self.entries.iter().find(|entry| entry.appid == appid)
    }

    pub fn target(&self) -> AssetKind {
        self.target
    }

    pub fn tab(&self) -> PanelTab {
        self.tab
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub fn candidates(&self) -> &[DynamicImage] {
        &self.candidates
    }

    pub fn candidate_handles(&self) -> &[Handle] {
        &self.candidate_handles
    }

    pub fn sgdb_results(&self) -> Option<&SgdbResults> {
        self.sgdb.as_ref()
    }

    /// The current art of one slot of the selected entry.
    pub fn slot_art(&self, kind: AssetKind) -> &SlotArt {
        self.slot_art.get(&kind).unwrap_or(&SlotArt::Empty)
    }

    /// Modify needs a current image in the target slot.
    pub fn can_modify(&self) -> bool {
        crate::ui::panel::modify_enabled(self.slot_art(self.target))
    }

    pub fn prompt_editor(&self) -> &text_editor::Content {
        &self.prompt_editor
    }

    pub fn settings(&self) -> Option<&SettingsForm> {
        self.settings.as_ref()
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    pub fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    pub fn steam_root(&self) -> Option<&Path> {
        self.steam_root.as_deref()
    }

    pub fn account_id(&self) -> Option<u32> {
        self.account_id
    }

    /// The task that loads the config and the library at start-up.
    fn start(&mut self) -> Task<Message> {
        self.busy = true;
        let input = StartupInput {
            env: Arc::clone(&self.services.env),
            config_path: self.services.config_path.clone(),
            steam_api_key: self.services.secrets.get_key(Service::SteamWeb),
            steam_api_base: self.services.steam_api_base.clone(),
        };
        Task::perform(load_library(input), |loaded| {
            Message::Loaded(Box::new(loaded))
        })
    }

    fn apply_loaded(&mut self, loaded: LoadedLibrary) -> Task<Message> {
        self.config = loaded.config;
        self.steam_root = loaded.steam_root;
        self.account_id = loaded.account_id;
        self.entries = loaded.entries;
        self.status = loaded.status;
        self.busy = false;
        if self.selected_entry().is_none() {
            self.selected = None;
            self.drop_candidates();
            self.slot_art.clear();
            return Task::none();
        }
        // The Steam folder can change, so the slot art can change too.
        self.load_slot_art_task()
    }

    fn select(&mut self, appid: u32) -> Task<Message> {
        if !self.entries.iter().any(|entry| entry.appid == appid) {
            return Task::none();
        }
        self.selected = Some(appid);
        self.drop_candidates();
        self.status = None;
        self.load_slot_art_task()
    }

    /// Marks every slot as loading and returns the task that reads the
    /// current art of the selected entry.
    fn load_slot_art_task(&mut self) -> Task<Message> {
        let Some(entry) = self.selected_entry().cloned() else {
            return Task::none();
        };
        let Some(root) = self.steam_root.clone() else {
            self.slot_art.clear();
            return Task::none();
        };
        self.slot_art = AssetKind::ALL
            .into_iter()
            .map(|kind| (kind, SlotArt::Loading))
            .collect();
        let appid = entry.appid;
        let input = SlotArtInput {
            grid_dir: self
                .account_id
                .map(|account_id| grid::grid_dir(&root, account_id)),
            cache_dir: defaults::library_cache_dir(&root),
            cdn_base: self.services.steam_cdn_base.clone(),
            entry,
        };
        Task::perform(load_all_slot_art(input), move |art| {
            Message::SlotArtLoaded { appid, art }
        })
    }

    fn apply_slot_art(&mut self, appid: u32, art: Vec<(AssetKind, SlotArt)>) {
        // The user picked another game while the art loaded.
        if self.selected != Some(appid) {
            return;
        }
        self.slot_art.extend(art);
    }

    /// Forgets the candidates and what they were made for.
    fn drop_candidates(&mut self) {
        self.candidates.clear();
        self.candidate_handles.clear();
        self.candidate_context = None;
        self.sgdb = None;
    }

    fn set_target(&mut self, kind: AssetKind) {
        // The candidate grid belongs to the old slot.
        if kind != self.target {
            self.drop_candidates();
        }
        self.target = kind;
    }

    fn edit_prompt(&mut self, action: text_editor::Action) {
        self.prompt_editor.perform(action);
        self.prompt = self.prompt_editor.text();
    }

    fn set_prompt(&mut self, text: String) {
        self.prompt_editor = text_editor::Content::with_text(&text);
        self.prompt = text;
    }

    /// Opens the dialog and starts the task that looks for Codex.
    fn open_settings(&mut self) -> Task<Message> {
        self.settings = Some(SettingsForm::new(&self.services.secrets, &self.config));
        Task::perform(
            check_codex(self.services.codex.clone()),
            Message::CodexChecked,
        )
    }

    /// Writes the changed keys and the config, then loads the library
    /// again. A failure keeps the dialog open with the reason.
    fn save_settings(&mut self) -> Task<Message> {
        // The reload would clear the busy flag of a running request.
        if self.busy {
            return Task::none();
        }
        let Some(form) = self.settings.as_mut() else {
            return Task::none();
        };
        for (service, key) in form.changed_keys() {
            if let Err(err) = self.services.secrets.set_key(service, key) {
                form.error = Some(format!("Cannot save the key: {err}"));
                return Task::none();
            }
        }
        let Some(path) = self.services.config_path.as_deref() else {
            form.error = Some("Cannot save the settings: the app has no settings folder.".into());
            return Task::none();
        };
        if let Err(err) = form.to_config().save(path) {
            form.error = Some(format!("Cannot save the settings: {err}"));
            return Task::none();
        }
        self.settings = None;
        self.start()
    }

    fn fail(&mut self, text: impl Into<String>) {
        self.status = Some(Status::Error(text.into()));
    }

    /// Marks a request as running and drops the old candidates.
    fn begin_request(&mut self) {
        self.busy = true;
        self.drop_candidates();
        self.status = None;
    }

    /// Like `begin_request`. A Codex request also says that it is slow.
    fn begin_request_with(&mut self, backend: &ImageBackend) {
        self.begin_request();
        if let ImageBackend::Codex(_) = backend {
            let count = self.config.images_per_request;
            let noun = if count == 1 { "image" } else { "images" };
            self.status = Some(Status::Info(format!(
                "Codex makes {count} {noun}. This can take several minutes."
            )));
        }
    }

    /// The image source of the config, or a status line when the OpenAI API
    /// has no key.
    fn image_backend(&mut self) -> Option<ImageBackend> {
        match self.config.image_source {
            ImageSource::Codex => Some(ImageBackend::Codex(CodexClient::new(
                self.services.codex.clone(),
                self.services.codex_home.clone(),
            ))),
            ImageSource::OpenAiApi => {
                let Some(key) = self.services.secrets.get_key(Service::OpenAi) else {
                    self.fail("Set the OpenAI API key in Settings before you make art.");
                    return None;
                };
                Some(ImageBackend::Api {
                    key,
                    model: self.config.openai_model.clone(),
                    base: self.services.openai_base.clone(),
                })
            }
        }
    }

    fn apply_candidates(
        &mut self,
        appid: u32,
        kind: AssetKind,
        result: Result<Vec<DynamicImage>, String>,
    ) {
        self.busy = false;
        // The user moved on to another game or slot while the request ran.
        if self.selected_appid() != Some(appid) || self.target != kind {
            return;
        }
        match result {
            Err(message) => {
                self.drop_candidates();
                self.fail(message);
            }
            Ok(images) if images.is_empty() => {
                self.drop_candidates();
                self.status = Some(Status::Info(
                    "No candidates came back. Try again or change the request.".to_string(),
                ));
            }
            Ok(images) => {
                let count = images.len();
                self.candidate_handles = images.iter().map(image_handle).collect();
                self.candidates = images;
                self.candidate_context = Some((appid, kind));
                self.status = Some(Status::Info(format!(
                    "Candidates: {count}. Apply one to save it."
                )));
            }
        }
    }

    fn apply_candidate(&mut self, index: usize) {
        let Some(image) = self.candidates.get(index).cloned() else {
            self.fail("Pick a candidate before you apply it.");
            return;
        };
        let Some((appid, kind)) = self.candidate_context else {
            self.fail("Pick a candidate before you apply it.");
            return;
        };
        self.write_candidate(appid, kind, &image);
    }

    /// Writes `image` to the `kind` slot of `appid` and shows it there.
    fn write_candidate(&mut self, appid: u32, kind: AssetKind, image: &DynamicImage) {
        let Some(grid) = self.grid_dir() else {
            return;
        };
        // Write with the slot the image was made for, not the slot the user
        // happens to look at now.
        match grid::write_asset(&grid, appid, kind, image) {
            Ok(path) => {
                if self.selected == Some(appid) {
                    self.slot_art
                        .insert(kind, SlotArt::Custom(image_handle(image)));
                }
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.status = Some(Status::Info(format!(
                    "Saved {name}. Restart Steam to show it."
                )));
            }
            Err(err) => self.fail(format!("Cannot save the art: {err}")),
        }
    }

    /// The grid folder of the current account, or a status line when the
    /// app knows no Steam folder or user.
    fn grid_dir(&mut self) -> Option<PathBuf> {
        match (self.steam_root.clone(), self.account_id) {
            (Some(root), Some(account_id)) => Some(grid::grid_dir(&root, account_id)),
            _ => {
                self.fail("Cannot find the Steam user folder. Set the Steam folder in Settings.");
                None
            }
        }
    }

    /// Runs the request of the active tab.
    fn start_action(&mut self) -> Task<Message> {
        // A running call owns the busy flag; a second press must not pay
        // for a second request.
        if self.busy {
            return Task::none();
        }
        let Some(entry) = self.selected_entry().cloned() else {
            self.fail("Select a game before you make art.");
            return Task::none();
        };
        let kind = self.target;
        let appid = entry.appid;
        match self.tab {
            PanelTab::SteamGridDb => {
                let Some(key) = self.services.secrets.get_key(Service::SteamGridDb) else {
                    self.fail("Set the SteamGridDB API key in Settings before you fetch art.");
                    return Task::none();
                };
                self.begin_request();
                self.list_task(key, entry, kind, 0)
            }
            PanelTab::Generate => {
                let Some(backend) = self.image_backend() else {
                    return Task::none();
                };
                self.begin_request_with(&backend);
                Task::perform(
                    request_generate(
                        backend,
                        self.prompt.clone(),
                        kind,
                        self.config.images_per_request,
                    ),
                    move |result| Message::CandidatesArrived {
                        appid,
                        kind,
                        result,
                    },
                )
            }
            PanelTab::Modify => {
                if !self.can_modify() {
                    self.fail("This slot has no current art to modify. Apply art first.");
                    return Task::none();
                }
                let Some(backend) = self.image_backend() else {
                    return Task::none();
                };
                let request = match self.modify_request(backend, entry, kind) {
                    Some(request) => request,
                    None => return Task::none(),
                };
                self.begin_request_with(&request.backend);
                Task::perform(request_modify(request), move |result| {
                    Message::CandidatesArrived {
                        appid,
                        kind,
                        result,
                    }
                })
            }
        }
    }

    /// The task that lists one API page of SteamGridDB images.
    fn list_task(
        &self,
        key: String,
        entry: LibraryEntry,
        kind: AssetKind,
        api_page: u32,
    ) -> Task<Message> {
        let appid = entry.appid;
        Task::perform(
            list_sgdb(key, self.services.sgdb_base.clone(), entry, kind, api_page),
            move |result| Message::SgdbListed {
                appid,
                kind,
                api_page,
                result,
            },
        )
    }

    fn apply_listed(
        &mut self,
        appid: u32,
        kind: AssetKind,
        api_page: u32,
        result: Result<Vec<Candidate>, String>,
    ) -> Task<Message> {
        self.busy = false;
        // The user moved on to another game or slot while the request ran.
        if self.selected_appid() != Some(appid) || self.target != kind {
            return Task::none();
        }
        let found = match result {
            Ok(found) => found,
            Err(message) => {
                // Next must not ask for the failed API page again and again.
                if let Some(results) = self.sgdb.as_mut() {
                    results.more = false;
                }
                self.fail(message);
                return Task::none();
            }
        };
        if api_page == 0 {
            if found.is_empty() {
                self.status = Some(Status::Info(
                    "No candidates came back. Try again or change the request.".to_string(),
                ));
                return Task::none();
            }
            self.sgdb = Some(SgdbResults::new(appid, kind, found));
        } else {
            let Some(results) = self.sgdb.as_mut() else {
                return Task::none();
            };
            results.append(api_page, found);
            // The Next press that asked for this API page moves on now.
            if results.first_index() + GRID_CELLS < results.found.len() {
                results.page += 1;
            }
        }
        self.show_sgdb_count();
        self.thumbs_task()
    }

    fn show_sgdb_count(&mut self) {
        if let Some(results) = &self.sgdb {
            let plus = if results.more { "+" } else { "" };
            self.status = Some(Status::Info(format!(
                "Candidates: {}{plus}. Apply one to save it.",
                results.found.len()
            )));
        }
    }

    /// The task that loads the missing thumbnails of the SteamGridDB page.
    fn thumbs_task(&self) -> Task<Message> {
        let Some(results) = &self.sgdb else {
            return Task::none();
        };
        let missing = results.thumbs_to_load();
        if missing.is_empty() {
            return Task::none();
        }
        let (appid, kind) = (results.appid, results.kind);
        Task::perform(
            load_thumbs(self.services.sgdb_base.clone(), missing),
            move |thumbs| Message::SgdbThumbsLoaded {
                appid,
                kind,
                thumbs,
            },
        )
    }

    fn apply_thumbs(&mut self, appid: u32, kind: AssetKind, thumbs: Vec<(u64, Option<Handle>)>) {
        if let Some(results) = self.sgdb.as_mut()
            && results.appid == appid
            && results.kind == kind
        {
            results.thumbs.extend(thumbs);
        }
    }

    fn previous_page(&mut self) -> Task<Message> {
        match self.sgdb.as_mut() {
            Some(results) if !self.busy && results.has_previous() => results.page -= 1,
            _ => return Task::none(),
        }
        self.thumbs_task()
    }

    fn next_page(&mut self) -> Task<Message> {
        if self.busy {
            return Task::none();
        }
        let Some(results) = self.sgdb.as_mut() else {
            return Task::none();
        };
        if !results.has_next() {
            return Task::none();
        }
        if !results.next_needs_fetch() {
            results.page += 1;
            return self.thumbs_task();
        }
        let api_page = results.api_page + 1;
        let Some(entry) = self.selected_entry().cloned() else {
            return Task::none();
        };
        let Some(key) = self.services.secrets.get_key(Service::SteamGridDb) else {
            self.fail("Set the SteamGridDB API key in Settings before you fetch art.");
            return Task::none();
        };
        let kind = self.target;
        self.busy = true;
        self.list_task(key, entry, kind, api_page)
    }

    /// Downloads the full image of one SteamGridDB candidate. The reply
    /// writes it.
    fn apply_sgdb(&mut self, index: usize) -> Task<Message> {
        if self.busy {
            return Task::none();
        }
        let Some(results) = &self.sgdb else {
            return Task::none();
        };
        let Some(candidate) = results.found.get(index).cloned() else {
            self.fail("Pick a candidate before you apply it.");
            return Task::none();
        };
        let (appid, kind) = (results.appid, results.kind);
        self.busy = true;
        self.status = Some(Status::Info("Downloading the full image...".to_string()));
        Task::perform(
            download_sgdb(self.services.sgdb_base.clone(), candidate, kind),
            move |result| Message::SgdbDownloaded {
                appid,
                kind,
                result,
            },
        )
    }

    fn apply_downloaded(
        &mut self,
        appid: u32,
        kind: AssetKind,
        result: Result<DynamicImage, String>,
    ) {
        self.busy = false;
        match result {
            Ok(image) => self.write_candidate(appid, kind, &image),
            Err(message) => self.fail(format!("Cannot download the image: {message}")),
        }
    }

    /// Everything the Modify request needs, or a status line when the app
    /// knows no Steam folder or user.
    fn modify_request(
        &mut self,
        backend: ImageBackend,
        entry: LibraryEntry,
        kind: AssetKind,
    ) -> Option<ModifyRequest> {
        let (Some(root), Some(account_id)) = (self.steam_root.clone(), self.account_id) else {
            self.fail("Cannot find the Steam user folder. Set the Steam folder in Settings.");
            return None;
        };
        Some(ModifyRequest {
            backend,
            cdn_base: self.services.steam_cdn_base.clone(),
            cache_dir: defaults::library_cache_dir(&root),
            custom: grid::current_custom(&grid::grid_dir(&root, account_id), entry.appid, kind),
            kind,
            count: self.config.images_per_request,
            prompt: self.prompt.clone(),
            entry,
        })
    }
}

pub fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::Loaded(loaded) => return state.apply_loaded(*loaded),
        Message::FilterChanged(text) => state.filter = text,
        Message::EntrySelected(appid) => return state.select(appid),
        Message::TargetSelected(kind) => state.set_target(kind),
        Message::TabSelected(tab) => state.tab = tab,
        Message::PromptChanged(text) => state.set_prompt(text),
        Message::PromptEdited(action) => state.edit_prompt(action),
        Message::SlotArtLoaded { appid, art } => state.apply_slot_art(appid, art),
        Message::SettingsOpened => return state.open_settings(),
        Message::ImageSourceSelected(source) => {
            if let Some(form) = state.settings.as_mut() {
                form.image_source = source;
            }
        }
        Message::ImagesPerRequestSelected(count) => {
            if let Some(form) = state.settings.as_mut() {
                form.images_per_request = count;
            }
        }
        Message::CodexChecked(status) => {
            if let Some(form) = state.settings.as_mut() {
                form.codex = status;
            }
        }
        Message::SettingsClosed => state.settings = None,
        Message::SettingsChanged(field, value) => {
            if let Some(form) = state.settings.as_mut() {
                form.set(field, value);
            }
        }
        Message::SettingsSaved => return state.save_settings(),
        Message::ActionPressed => return state.start_action(),
        Message::CandidatesArrived {
            appid,
            kind,
            result,
        } => state.apply_candidates(appid, kind, result),
        Message::ApplyPressed(index) if state.sgdb.is_some() => return state.apply_sgdb(index),
        Message::ApplyPressed(index) => state.apply_candidate(index),
        Message::SgdbListed {
            appid,
            kind,
            api_page,
            result,
        } => return state.apply_listed(appid, kind, api_page, result),
        Message::SgdbThumbsLoaded {
            appid,
            kind,
            thumbs,
        } => state.apply_thumbs(appid, kind, thumbs),
        Message::SgdbPrevious => return state.previous_page(),
        Message::SgdbNext => return state.next_page(),
        Message::SgdbDownloaded {
            appid,
            kind,
            result,
        } => state.apply_downloaded(appid, kind, result),
    }
    Task::none()
}

/// Loads the config, the Steam root, the account and the library.
pub async fn load_library(input: StartupInput) -> LoadedLibrary {
    let mut errors: Vec<String> = Vec::new();
    let mut config = Config::default();
    if let Some(path) = &input.config_path {
        match Config::load(path) {
            Ok(loaded) => config = loaded,
            Err(err) => errors.push(format!("Cannot load the settings: {err}")),
        }
    }

    let Some(steam_root) = paths::find_steam_root(input.env.as_ref(), config.steam_root.as_deref())
    else {
        errors.push("Cannot find the Steam folder. Set it in Settings.".to_string());
        return LoadedLibrary {
            config,
            steam_root: None,
            account_id: None,
            entries: Vec::new(),
            status: joined_status(errors),
        };
    };

    let mut user: Option<SteamUser> = None;
    match users::most_recent_user(&steam_root) {
        Ok(Some(found)) => user = Some(found),
        Ok(None) => errors.push("Cannot find a Steam user in loginusers.vdf.".to_string()),
        Err(err) => errors.push(format!("Cannot read the Steam user: {err}")),
    }
    let account_id = user.map(|user| user.account_id);

    let mut entries = match crate::steam::library::installed_games(&steam_root) {
        Ok(entries) => entries,
        Err(err) => {
            errors.push(format!("Cannot read the installed games: {err}"));
            Vec::new()
        }
    };
    if let Some(account_id) = account_id {
        match shortcuts::load_shortcuts(&steam_root, account_id) {
            Ok(found) => entries.extend(found),
            Err(err) => errors.push(format!("Cannot read the non-Steam shortcuts: {err}")),
        }
    }
    sort_entries(&mut entries);

    if let (Some(key), Some(user)) = (input.steam_api_key, user) {
        match owned::fetch_owned(&key, user.steam_id64, &input.steam_api_base).await {
            Ok(owned) => entries = owned::merge(entries, owned),
            Err(err) => errors.push(format!("Cannot list owned games: {err}")),
        }
    }

    LoadedLibrary {
        config,
        steam_root: Some(steam_root),
        account_id,
        entries,
        status: joined_status(errors),
    }
}

/// One status line for every start-up problem.
fn joined_status(errors: Vec<String>) -> Option<Status> {
    (!errors.is_empty()).then(|| Status::Error(errors.join(" ")))
}

/// One API page of the images SteamGridDB offers for one slot.
async fn list_sgdb(
    key: String,
    base: String,
    entry: LibraryEntry,
    kind: AssetKind,
    api_page: u32,
) -> Result<Vec<Candidate>, String> {
    SgdbClient::new(&key, &base)
        .candidates_page(&entry, kind, api_page)
        .await
        .map_err(|err| err.to_string())
}

/// The thumbnails of `candidates` by ID; `None` for one that failed.
async fn load_thumbs(base: String, candidates: Vec<Candidate>) -> Vec<(u64, Option<Handle>)> {
    // Image URLs need no key.
    let client = SgdbClient::new("", &base);
    let mut thumbs = Vec::new();
    for candidate in candidates {
        let handle = client
            .thumbnail(&candidate)
            .await
            .ok()
            .map(Handle::from_bytes);
        thumbs.push((candidate.id, handle));
    }
    thumbs
}

/// The full image of `candidate`, fitted to `kind`.
async fn download_sgdb(
    base: String,
    candidate: Candidate,
    kind: AssetKind,
) -> Result<DynamicImage, String> {
    // Image URLs need no key.
    SgdbClient::new("", &base)
        .download(&candidate, kind)
        .await
        .map_err(|err| err.to_string())
}

/// Where Generate and Modify get their images.
enum ImageBackend {
    Api {
        key: String,
        model: String,
        base: String,
    },
    Codex(CodexClient),
}

async fn check_codex(runner: Arc<dyn CodexRunner>) -> CodexStatus {
    crate::codex::check(runner.as_ref()).await
}

async fn request_generate(
    backend: ImageBackend,
    prompt: String,
    kind: AssetKind,
    count: u8,
) -> Result<Vec<DynamicImage>, String> {
    match backend {
        ImageBackend::Api { key, model, base } => OpenAiClient::new(&key, &model, &base)
            .generate(&prompt, kind, count)
            .await
            .map_err(|err| err.to_string()),
        ImageBackend::Codex(client) => client
            .generate(&prompt, kind, count)
            .await
            .map_err(|err| err.to_string()),
    }
}

/// Everything the Modify request reads.
struct ModifyRequest {
    backend: ImageBackend,
    cdn_base: String,
    cache_dir: PathBuf,
    custom: Option<PathBuf>,
    kind: AssetKind,
    /// How many images to ask for.
    count: u8,
    prompt: String,
    entry: LibraryEntry,
}

async fn request_modify(request: ModifyRequest) -> Result<Vec<DynamicImage>, String> {
    let source = current_image(&request).await?;
    match &request.backend {
        ImageBackend::Api { key, model, base } => OpenAiClient::new(key, model, base)
            .modify(&source, &request.prompt, request.kind, request.count)
            .await
            .map_err(|err| err.to_string()),
        ImageBackend::Codex(client) => client
            .modify(&source, &request.prompt, request.kind, request.count)
            .await
            .map_err(|err| err.to_string()),
    }
}

/// The image Modify starts from: the custom art, else Steam's own art.
async fn current_image(request: &ModifyRequest) -> Result<DynamicImage, String> {
    if let Some(path) = &request.custom {
        return image::open(path).map_err(|err| format!("Cannot read {}: {err}", path.display()));
    }
    let art = defaults::default_art(
        &request.cache_dir,
        &request.cdn_base,
        request.entry.appid,
        request.entry.source,
        request.kind,
    )
    .await
    .map_err(|err| err.to_string())?;
    match art {
        Some(DefaultArt::Cached(path)) => {
            image::open(&path).map_err(|err| format!("Cannot read {}: {err}", path.display()))
        }
        Some(DefaultArt::Downloaded(bytes)) => image::load_from_memory(&bytes)
            .map_err(|err| format!("The default art is not a valid image: {err}")),
        None => Err("There is no current art to modify. Apply art first.".to_string()),
    }
}

/// The image the view shows for a decoded image.
fn image_handle(image: &DynamicImage) -> Handle {
    let rgba = image.to_rgba8();
    Handle::from_rgba(rgba.width(), rgba.height(), rgba.into_raw())
}

/// Where the current art of the slots of one entry lives.
pub struct SlotArtInput {
    /// The grid folder; `None` when the app knows no Steam user.
    pub grid_dir: Option<PathBuf>,
    pub cache_dir: PathBuf,
    pub cdn_base: String,
    pub entry: LibraryEntry,
}

async fn load_all_slot_art(input: SlotArtInput) -> Vec<(AssetKind, SlotArt)> {
    let mut art = Vec::new();
    for kind in AssetKind::ALL {
        art.push((kind, load_slot_art(&input, kind).await));
    }
    art
}

/// The art one slot shows: the custom file, else Steam's own art, else
/// nothing. A file that cannot be read counts as no art.
pub async fn load_slot_art(input: &SlotArtInput, kind: AssetKind) -> SlotArt {
    let appid = input.entry.appid;
    let custom = input
        .grid_dir
        .as_deref()
        .and_then(|grid| grid::current_custom(grid, appid, kind));
    if let Some(path) = custom
        && let Ok(bytes) = tokio::fs::read(&path).await
    {
        return SlotArt::Custom(Handle::from_bytes(bytes));
    }
    let art = defaults::default_art(
        &input.cache_dir,
        &input.cdn_base,
        appid,
        input.entry.source,
        kind,
    )
    .await;
    let bytes = match art {
        Ok(Some(DefaultArt::Cached(path))) => tokio::fs::read(&path).await.ok(),
        Ok(Some(DefaultArt::Downloaded(bytes))) => Some(bytes),
        Ok(None) | Err(_) => None,
    };
    match bytes {
        Some(bytes) => SlotArt::Default(Handle::from_bytes(bytes)),
        None => SlotArt::Empty,
    }
}

pub fn view(state: &State) -> Element<'_, Message> {
    crate::ui::view(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;
    use std::sync::Mutex;

    use image::{Rgba, RgbaImage};
    use tempfile::TempDir;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::codex::{BoxFuture, CodexRunner, CodexStatus, RunOutput};
    use crate::config::ImageSource;
    use crate::secrets::{KeyStore, SecretsError};
    use crate::steam::library::EntrySource;
    use crate::steam::paths::Os;
    use crate::ui::settings::SettingsField;

    const ACCOUNT_ID: u32 = 39_734_273;

    #[derive(Default, Clone)]
    struct FakeStore {
        keys: Arc<Mutex<HashMap<&'static str, String>>>,
    }

    impl KeyStore for FakeStore {
        fn get(&self, service: Service) -> Result<Option<String>, SecretsError> {
            Ok(self
                .keys
                .lock()
                .unwrap()
                .get(service.keyring_user())
                .cloned())
        }
        fn set(&self, service: Service, key: &str) -> Result<(), SecretsError> {
            self.keys
                .lock()
                .unwrap()
                .insert(service.keyring_user(), key.to_string());
            Ok(())
        }
        fn delete(&self, service: Service) -> Result<(), SecretsError> {
            self.keys.lock().unwrap().remove(service.keyring_user());
            Ok(())
        }
    }

    struct FakeEnv {
        home: PathBuf,
    }

    impl PathEnv for FakeEnv {
        fn os(&self) -> Os {
            Os::Linux
        }
        fn home_dir(&self) -> Option<PathBuf> {
            Some(self.home.clone())
        }
        fn registry_steam_path(&self) -> Option<PathBuf> {
            None
        }
        fn program_files_x86(&self) -> Option<PathBuf> {
            None
        }
    }

    /// A Codex that is installed and saves one PNG of `size` per run.
    struct FakeCodex {
        home: PathBuf,
        size: (u32, u32),
        runs: std::sync::atomic::AtomicUsize,
    }

    impl CodexRunner for FakeCodex {
        fn version(&self) -> BoxFuture<'_, Option<String>> {
            Box::pin(async { Some("codex-cli 0.167.0".to_string()) })
        }
        fn exec(
            &self,
            _args: Vec<std::ffi::OsString>,
        ) -> BoxFuture<'_, std::io::Result<RunOutput>> {
            let run = self.runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let folder = self.home.join(format!("generated_images/t{run}"));
            let (width, height) = self.size;
            Box::pin(async move {
                fs::create_dir_all(&folder).unwrap();
                fs::write(folder.join("ig.png"), png_bytes(width, height)).unwrap();
                Ok(RunOutput {
                    success: true,
                    stdout: format!("{{\"type\":\"thread.started\",\"thread_id\":\"t{run}\"}}"),
                    stderr: String::new(),
                })
            })
        }
    }

    /// No Codex on the `PATH`.
    struct NoCodex;

    impl CodexRunner for NoCodex {
        fn version(&self) -> BoxFuture<'_, Option<String>> {
            Box::pin(async { None })
        }
        fn exec(
            &self,
            _args: Vec<std::ffi::OsString>,
        ) -> BoxFuture<'_, std::io::Result<RunOutput>> {
            Box::pin(async { Err(std::io::Error::from(std::io::ErrorKind::NotFound)) })
        }
    }

    /// Services with a fake key store, no environment keys and no server.
    fn services(keys: &[(Service, &str)]) -> Services {
        let store = FakeStore::default();
        for (service, key) in keys {
            store.set(*service, key).unwrap();
        }
        Services {
            secrets: Secrets::new(
                Box::new(store),
                Box::new(|_: &str| None::<String>) as crate::secrets::EnvLookup,
            ),
            env: Arc::new(FakeEnv {
                home: PathBuf::from("/nonexistent-home"),
            }),
            config_path: None,
            steam_api_base: "http://127.0.0.1:1".to_string(),
            sgdb_base: "http://127.0.0.1:1".to_string(),
            openai_base: "http://127.0.0.1:1".to_string(),
            steam_cdn_base: "http://127.0.0.1:1".to_string(),
            codex: Arc::new(NoCodex),
            codex_home: None,
        }
    }

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/steam")
    }

    fn library_a() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/libraries/a")
    }

    /// A Steam root with a recent user, the shortcuts fixture and two
    /// installed games.
    fn temp_steam_root() -> TempDir {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("config")).unwrap();
        fs::copy(
            fixture_root().join("config/loginusers.vdf"),
            root.join("config/loginusers.vdf"),
        )
        .unwrap();
        let account = root.join("userdata").join(ACCOUNT_ID.to_string());
        fs::create_dir_all(account.join("config")).unwrap();
        fs::copy(
            fixture_root()
                .join("userdata")
                .join(ACCOUNT_ID.to_string())
                .join("config/shortcuts.vdf"),
            account.join("config/shortcuts.vdf"),
        )
        .unwrap();
        fs::create_dir_all(root.join("steamapps")).unwrap();
        fs::write(
            root.join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                library_a().display().to_string().replace('\\', "\\\\")
            ),
        )
        .unwrap();
        temp
    }

    /// A config file that points at `steam_root`.
    fn config_file(steam_root: &Path) -> TempDir {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("config.toml");
        Config {
            steam_root: Some(steam_root.to_path_buf()),
            openai_model: crate::config::DEFAULT_OPENAI_MODEL.to_string(),
            ..Config::default()
        }
        .save(&path)
        .unwrap();
        temp
    }

    fn startup_input(
        config_path: Option<PathBuf>,
        steam_api_key: Option<String>,
        steam_api_base: &str,
    ) -> StartupInput {
        StartupInput {
            env: Arc::new(FakeEnv {
                home: PathBuf::from("/nonexistent-home"),
            }),
            config_path,
            steam_api_key,
            steam_api_base: steam_api_base.to_string(),
        }
    }

    fn steam_entry(appid: u32, name: &str) -> LibraryEntry {
        LibraryEntry {
            appid,
            name: name.to_string(),
            source: EntrySource::Steam,
            installed: true,
        }
    }

    fn solid(width: u32, height: u32) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(width, height, Rgba([9, 9, 9, 255])))
    }

    /// The message a request task sends back with images.
    fn candidates(appid: u32, kind: AssetKind, images: Vec<DynamicImage>) -> Message {
        Message::CandidatesArrived {
            appid,
            kind,
            result: Ok(images),
        }
    }

    /// A state with a loaded library, as the start-up task would set it.
    fn state_with(
        services: Services,
        steam_root: Option<PathBuf>,
        entries: Vec<LibraryEntry>,
    ) -> State {
        let mut state = State::new(services);
        send(
            &mut state,
            Message::Loaded(Box::new(LoadedLibrary {
                config: Config::default(),
                steam_root,
                account_id: Some(ACCOUNT_ID),
                entries,
                status: None,
            })),
        );
        state
    }

    /// Sends one message and drops the task it starts.
    fn send(state: &mut State, message: Message) {
        drop(update(state, message));
    }

    /// Picks the target slot and the panel tab, as the user does in the panel.
    fn pick(state: &mut State, kind: AssetKind, tab: PanelTab) {
        send(state, Message::TargetSelected(kind));
        send(state, Message::TabSelected(tab));
    }

    fn state_names(state: &State) -> Vec<String> {
        state
            .visible_entries()
            .into_iter()
            .map(|entry| entry.name.clone())
            .collect()
    }

    fn entry_names(entries: &[LibraryEntry]) -> Vec<String> {
        entries.iter().map(|entry| entry.name.clone()).collect()
    }

    #[test]
    fn no_entry_is_selected_at_start() {
        let state = State::new(services(&[]));
        assert_eq!(state.selected_appid(), None);
        assert!(state.selected_entry().is_none());
        assert!(!state.busy());
        assert!(state.status().is_none());
    }

    #[test]
    fn select_entry_sets_the_selected_entry() {
        let mut state = state_with(
            services(&[]),
            None,
            vec![
                steam_entry(440, "Team Fortress 2"),
                steam_entry(570, "Dota 2"),
            ],
        );
        send(&mut state, Message::EntrySelected(570));
        assert_eq!(state.selected_appid(), Some(570));
        assert_eq!(
            state.selected_entry().map(|entry| entry.name.as_str()),
            Some("Dota 2")
        );
    }

    #[test]
    fn unknown_entry_does_not_change_the_selection() {
        let mut state = state_with(
            services(&[]),
            None,
            vec![steam_entry(440, "Team Fortress 2")],
        );
        send(&mut state, Message::EntrySelected(999));
        assert_eq!(state.selected_appid(), None);
    }

    #[test]
    fn filter_text_narrows_the_list() {
        let mut state = state_with(
            services(&[]),
            None,
            vec![
                steam_entry(440, "Team Fortress 2"),
                steam_entry(570, "Dota 2"),
                steam_entry(620, "Portal 2"),
            ],
        );
        send(&mut state, Message::FilterChanged("PORT".to_string()));
        assert_eq!(state_names(&state), ["Portal 2"]);
        send(&mut state, Message::FilterChanged("  team  ".to_string()));
        assert_eq!(state_names(&state), ["Team Fortress 2"]);
        send(&mut state, Message::FilterChanged("   ".to_string()));
        assert_eq!(state_names(&state).len(), 3);
    }

    #[test]
    fn apply_writes_the_candidate_to_the_grid_folder() {
        let temp = TempDir::new().unwrap();
        let mut state = state_with(
            services(&[]),
            Some(temp.path().to_path_buf()),
            vec![steam_entry(440, "Team Fortress 2")],
        );
        send(&mut state, Message::EntrySelected(440));
        pick(&mut state, AssetKind::Hero, PanelTab::SteamGridDb);
        send(
            &mut state,
            candidates(440, AssetKind::Hero, vec![solid(1920, 620)]),
        );
        send(&mut state, Message::ApplyPressed(0));

        let written = temp
            .path()
            .join("userdata")
            .join(ACCOUNT_ID.to_string())
            .join("config/grid/440_hero.png");
        assert!(written.is_file(), "{}", written.display());
        let image = image::open(&written).unwrap();
        assert_eq!((image.width(), image.height()), (1920, 620));

        let status = state.status().expect("a status line");
        assert!(!status.is_error(), "{}", status.text());
        assert!(status.text().contains("440_hero.png"), "{}", status.text());
    }

    #[test]
    fn apply_uses_the_kind_the_candidates_were_made_for() {
        let temp = TempDir::new().unwrap();
        let mut state = state_with(
            services(&[]),
            Some(temp.path().to_path_buf()),
            vec![steam_entry(440, "Team Fortress 2")],
        );
        send(&mut state, Message::EntrySelected(440));
        pick(&mut state, AssetKind::Hero, PanelTab::SteamGridDb);
        send(
            &mut state,
            candidates(440, AssetKind::Hero, vec![solid(1920, 620)]),
        );
        // A target change must not move the Hero image into the portrait slot.
        state.target = AssetKind::Portrait;
        send(&mut state, Message::ApplyPressed(0));

        let grid = temp
            .path()
            .join("userdata")
            .join(ACCOUNT_ID.to_string())
            .join("config/grid");
        assert!(grid.join("440_hero.png").is_file());
        assert!(!grid.join("440p.png").exists());
    }

    #[test]
    fn a_target_change_clears_the_candidates() {
        let temp = TempDir::new().unwrap();
        let mut state = state_with(
            services(&[]),
            Some(temp.path().to_path_buf()),
            vec![steam_entry(440, "Team Fortress 2")],
        );
        send(&mut state, Message::EntrySelected(440));
        pick(&mut state, AssetKind::Hero, PanelTab::SteamGridDb);
        send(
            &mut state,
            candidates(440, AssetKind::Hero, vec![solid(1920, 620)]),
        );
        pick(&mut state, AssetKind::Portrait, PanelTab::SteamGridDb);
        assert!(state.candidates().is_empty());

        send(&mut state, Message::ApplyPressed(0));
        let grid = temp
            .path()
            .join("userdata")
            .join(ACCOUNT_ID.to_string())
            .join("config/grid");
        assert!(!grid.join("440p.png").exists());
        let status = state.status().expect("a status line");
        assert!(status.is_error(), "{}", status.text());
    }

    #[test]
    fn reply_for_another_game_is_dropped() {
        let mut state = state_with(
            services(&[(Service::SteamGridDb, "sgdb-key")]),
            Some(PathBuf::from("/nonexistent-steam")),
            vec![
                steam_entry(440, "Team Fortress 2"),
                steam_entry(570, "Dota 2"),
            ],
        );
        send(&mut state, Message::EntrySelected(440));
        pick(&mut state, AssetKind::Hero, PanelTab::SteamGridDb);
        send(&mut state, Message::ActionPressed);
        assert!(state.busy());

        // The user moves on to another game while the request runs.
        send(&mut state, Message::EntrySelected(570));
        send(
            &mut state,
            candidates(440, AssetKind::Hero, vec![solid(1920, 620)]),
        );

        assert!(!state.busy());
        assert!(state.candidates().is_empty());
    }

    #[test]
    fn reply_for_another_kind_is_dropped() {
        let mut state = state_with(
            services(&[]),
            None,
            vec![steam_entry(440, "Team Fortress 2")],
        );
        send(&mut state, Message::EntrySelected(440));
        pick(&mut state, AssetKind::Portrait, PanelTab::SteamGridDb);
        send(
            &mut state,
            candidates(440, AssetKind::Hero, vec![solid(1920, 620)]),
        );
        assert!(state.candidates().is_empty());
    }

    #[test]
    fn apply_without_a_candidate_shows_an_error() {
        let temp = TempDir::new().unwrap();
        let mut state = state_with(
            services(&[]),
            Some(temp.path().to_path_buf()),
            vec![steam_entry(440, "Team Fortress 2")],
        );
        send(&mut state, Message::EntrySelected(440));
        send(&mut state, Message::ApplyPressed(3));

        let status = state.status().expect("a status line");
        assert!(status.is_error(), "{}", status.text());
        assert!(status.text().contains("candidate"), "{}", status.text());
    }

    #[test]
    fn missing_steamgriddb_key_shows_a_clear_message() {
        let mut state = state_with(
            services(&[]),
            Some(PathBuf::from("/nonexistent-steam")),
            vec![steam_entry(440, "Team Fortress 2")],
        );
        send(&mut state, Message::EntrySelected(440));
        pick(&mut state, AssetKind::Hero, PanelTab::SteamGridDb);
        send(&mut state, Message::ActionPressed);

        assert!(!state.busy());
        assert!(state.candidates().is_empty());
        let status = state.status().expect("a status line");
        assert!(status.is_error(), "{}", status.text());
        assert!(
            status.text().contains("SteamGridDB API key"),
            "{}",
            status.text()
        );
        assert!(
            status.text().to_lowercase().contains("settings"),
            "{}",
            status.text()
        );
    }

    #[test]
    fn missing_openai_key_shows_a_clear_message() {
        let mut state = state_with(
            services(&[]),
            Some(PathBuf::from("/nonexistent-steam")),
            vec![steam_entry(440, "Team Fortress 2")],
        );
        send(&mut state, Message::EntrySelected(440));
        pick(&mut state, AssetKind::Wide, PanelTab::Generate);
        send(&mut state, Message::ActionPressed);

        assert!(!state.busy());
        let status = state.status().expect("a status line");
        assert!(status.is_error(), "{}", status.text());
        assert!(
            status.text().contains("OpenAI API key"),
            "{}",
            status.text()
        );
    }

    #[test]
    fn api_error_shows_in_the_status_line() {
        let mut state = state_with(
            services(&[]),
            None,
            vec![steam_entry(440, "Team Fortress 2")],
        );
        send(&mut state, Message::EntrySelected(440));
        pick(&mut state, AssetKind::Hero, PanelTab::Generate);
        send(
            &mut state,
            Message::CandidatesArrived {
                appid: 440,
                kind: AssetKind::Hero,
                result: Err("OpenAI returned HTTP 400: no dice".to_string()),
            },
        );
        assert!(state.candidates().is_empty());
        let status = state.status().expect("a status line");
        assert!(status.is_error(), "{}", status.text());
        assert_eq!(status.text(), "OpenAI returned HTTP 400: no dice");
    }

    #[test]
    fn empty_candidates_show_a_message_instead_of_an_empty_grid() {
        let mut state = state_with(
            services(&[]),
            None,
            vec![steam_entry(440, "Team Fortress 2")],
        );
        send(&mut state, Message::EntrySelected(440));
        pick(&mut state, AssetKind::Hero, PanelTab::SteamGridDb);
        send(&mut state, candidates(440, AssetKind::Hero, Vec::new()));
        assert!(state.candidates().is_empty());
        let status = state.status().expect("a status line");
        assert!(!status.is_error(), "{}", status.text());
        assert!(status.text().contains("No candidates"), "{}", status.text());
    }

    #[test]
    fn request_sets_the_busy_flag_until_the_reply() {
        let mut state = state_with(
            services(&[(Service::SteamGridDb, "sgdb-key")]),
            Some(PathBuf::from("/nonexistent-steam")),
            vec![steam_entry(440, "Team Fortress 2")],
        );
        send(&mut state, Message::EntrySelected(440));
        pick(&mut state, AssetKind::Wide, PanelTab::SteamGridDb);
        send(&mut state, Message::ActionPressed);
        assert!(state.busy());
        assert!(state.candidates().is_empty());

        send(
            &mut state,
            candidates(440, AssetKind::Wide, vec![solid(920, 430)]),
        );
        assert!(!state.busy());
        assert_eq!(state.candidates().len(), 1);
        let status = state.status().expect("a status line");
        assert!(!status.is_error(), "{}", status.text());
        assert!(status.text().contains('1'), "{}", status.text());
    }

    #[test]
    fn request_without_a_selected_game_shows_an_error() {
        let mut state = state_with(
            services(&[(Service::OpenAi, "sk-key")]),
            None,
            vec![steam_entry(440, "Team Fortress 2")],
        );
        pick(&mut state, AssetKind::Logo, PanelTab::Generate);
        send(&mut state, Message::ActionPressed);
        assert!(!state.busy());
        let status = state.status().expect("a status line");
        assert!(status.is_error(), "{}", status.text());
        assert!(status.text().contains("Select a game"), "{}", status.text());
    }

    #[test]
    fn startup_sets_busy_until_the_library_loads() {
        let mut state = State::new(services(&[]));
        let task = state.start();
        assert!(state.busy());
        drop(task);

        send(
            &mut state,
            Message::Loaded(Box::new(LoadedLibrary {
                config: Config::default(),
                steam_root: Some(PathBuf::from("/steam")),
                account_id: Some(ACCOUNT_ID),
                entries: vec![steam_entry(440, "Team Fortress 2")],
                status: None,
            })),
        );
        assert!(!state.busy());
    }

    #[tokio::test]
    async fn startup_loads_installed_games_and_shortcuts() {
        let root = temp_steam_root();
        let config = config_file(root.path());
        let loaded = load_library(startup_input(
            Some(config.path().join("config.toml")),
            None,
            "http://127.0.0.1:1",
        ))
        .await;

        assert_eq!(loaded.steam_root.as_deref(), Some(root.path()));
        assert_eq!(loaded.account_id, Some(ACCOUNT_ID));
        assert_eq!(
            entry_names(&loaded.entries),
            [
                "Da Vinci Virus",
                "Dota 2",
                "Gibson Terminal",
                "Team Fortress 2",
            ]
        );
        assert!(loaded.status.is_none(), "{:?}", loaded.status);
    }

    #[tokio::test]
    async fn startup_owned_failure_keeps_installed_games() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/IPlayerService/GetOwnedGames/v1/"))
            .respond_with(ResponseTemplate::new(403))
            .expect(1)
            .mount(&server)
            .await;
        let root = temp_steam_root();
        let config = config_file(root.path());
        let loaded = load_library(startup_input(
            Some(config.path().join("config.toml")),
            Some("bad-key".to_string()),
            &server.uri(),
        ))
        .await;

        let status = loaded.status.as_ref().expect("a status line");
        assert!(status.is_error(), "{}", status.text());
        assert!(
            status.text().starts_with("Cannot list owned games:"),
            "{}",
            status.text()
        );
        assert!(
            status.text().contains("Steam Web API key"),
            "{}",
            status.text()
        );
        assert_eq!(
            entry_names(&loaded.entries),
            [
                "Da Vinci Virus",
                "Dota 2",
                "Gibson Terminal",
                "Team Fortress 2",
            ]
        );
    }

    #[tokio::test]
    async fn startup_merges_owned_games() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "response": {
                "game_count": 2,
                "games": [
                    {"appid": 440, "name": "Team Fortress 2"},
                    {"appid": 400, "name": "portal"}
                ]
            }
        });
        Mock::given(method("GET"))
            .and(path("/IPlayerService/GetOwnedGames/v1/"))
            .and(query_param("key", "web-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
        let root = temp_steam_root();
        let config = config_file(root.path());
        let loaded = load_library(startup_input(
            Some(config.path().join("config.toml")),
            Some("web-key".to_string()),
            &server.uri(),
        ))
        .await;

        assert_eq!(
            entry_names(&loaded.entries),
            [
                "Da Vinci Virus",
                "Dota 2",
                "Gibson Terminal",
                "portal",
                "Team Fortress 2",
            ]
        );
        let portal = loaded
            .entries
            .iter()
            .find(|entry| entry.appid == 400)
            .expect("portal");
        assert!(!portal.installed);
        let tf2 = loaded
            .entries
            .iter()
            .find(|entry| entry.appid == 440)
            .expect("Team Fortress 2");
        assert!(tf2.installed);
    }

    #[tokio::test]
    async fn startup_without_a_steam_user_shows_an_error() {
        let root = temp_steam_root();
        fs::write(
            root.path().join("config/loginusers.vdf"),
            "\"users\"\n{\n\t\"76561198000000001\"\n\t{\n\t\t\"AccountName\"\t\t\"acidburn\"\n\t}\n}\n",
        )
        .unwrap();
        let config = config_file(root.path());
        let loaded = load_library(startup_input(
            Some(config.path().join("config.toml")),
            None,
            "http://127.0.0.1:1",
        ))
        .await;

        assert_eq!(loaded.account_id, None);
        assert_eq!(entry_names(&loaded.entries), ["Dota 2", "Team Fortress 2"]);
        let status = loaded.status.as_ref().expect("a status line");
        assert!(status.is_error(), "{}", status.text());
        assert!(status.text().contains("Steam user"), "{}", status.text());
    }

    #[tokio::test]
    async fn startup_without_a_steam_root_shows_an_error() {
        let loaded = load_library(startup_input(None, None, "http://127.0.0.1:1")).await;
        assert!(loaded.steam_root.is_none());
        assert_eq!(loaded.account_id, None);
        assert!(loaded.entries.is_empty());
        let status = loaded.status.as_ref().expect("a status line");
        assert!(status.is_error(), "{}", status.text());
        assert!(status.text().contains("Steam folder"), "{}", status.text());
    }

    #[test]
    fn loaded_message_applies_the_library() {
        let mut state = State::new(services(&[]));
        send(
            &mut state,
            Message::Loaded(Box::new(LoadedLibrary {
                config: Config::default(),
                steam_root: Some(PathBuf::from("/steam")),
                account_id: Some(ACCOUNT_ID),
                entries: vec![steam_entry(440, "Team Fortress 2")],
                status: Some(Status::Info("owned games unavailable".to_string())),
            })),
        );
        assert_eq!(state.entries().len(), 1);
        assert_eq!(state.steam_root(), Some(Path::new("/steam")));
        assert_eq!(state.account_id(), Some(ACCOUNT_ID));
        assert_eq!(
            state.status().map(Status::text),
            Some("owned games unavailable")
        );

        send(&mut state, Message::EntrySelected(440));
        send(
            &mut state,
            Message::Loaded(Box::new(LoadedLibrary {
                config: Config::default(),
                steam_root: Some(PathBuf::from("/steam")),
                account_id: Some(ACCOUNT_ID),
                entries: vec![steam_entry(570, "Dota 2")],
                status: None,
            })),
        );
        assert_eq!(state.selected_appid(), None);
        assert!(state.status().is_none());
    }

    /// The bytes of a PNG at one size.
    fn png_bytes(width: u32, height: u32) -> Vec<u8> {
        use image::ImageFormat;
        use std::io::Cursor;

        let mut bytes = Cursor::new(Vec::new());
        solid(width, height)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    /// A SteamGridDB candidate with image URLs on `base`.
    fn sgdb_candidate(base: &str, id: u64) -> Candidate {
        Candidate {
            id,
            thumb_url: format!("{base}/thumb/{id}.png"),
            full_url: format!("{base}/full/{id}.png"),
        }
    }

    fn sgdb_candidates(base: &str, ids: std::ops::RangeInclusive<u64>) -> Vec<Candidate> {
        ids.map(|id| sgdb_candidate(base, id)).collect()
    }

    /// The message a SteamGridDB list request sends back.
    fn listed(appid: u32, kind: AssetKind, api_page: u32, found: Vec<Candidate>) -> Message {
        Message::SgdbListed {
            appid,
            kind,
            api_page,
            result: Ok(found),
        }
    }

    /// Team Fortress 2 selected, Hero on the SteamGridDB tab, and a search
    /// that found `found`.
    fn state_with_sgdb(steam_root: Option<PathBuf>, found: Vec<Candidate>) -> State {
        let mut state = state_with(
            services(&[(Service::SteamGridDb, "sgdb-key")]),
            steam_root,
            vec![
                steam_entry(440, "Team Fortress 2"),
                steam_entry(570, "Dota 2"),
            ],
        );
        send(&mut state, Message::EntrySelected(440));
        pick(&mut state, AssetKind::Hero, PanelTab::SteamGridDb);
        send(&mut state, Message::ActionPressed);
        send(&mut state, listed(440, AssetKind::Hero, 0, found));
        state
    }

    fn page_ids(state: &State) -> Vec<u64> {
        let results = state.sgdb_results().expect("SteamGridDB results");
        results.page_candidates().iter().map(|c| c.id).collect()
    }

    fn sgdb_list_body(base: &str, ids: std::ops::RangeInclusive<u64>) -> serde_json::Value {
        let data: Vec<_> = ids
            .map(|id| {
                serde_json::json!({
                    "id": id,
                    "url": format!("{base}/full/{id}.png"),
                    "thumb": format!("{base}/thumb/{id}.png"),
                })
            })
            .collect();
        serde_json::json!({"success": true, "data": data})
    }

    #[tokio::test]
    async fn sgdb_search_keeps_every_result_and_shows_the_first_page() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/heroes/steam/440"))
            .and(query_param("page", "0"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(sgdb_list_body(&server.uri(), 1..=6)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let found = list_sgdb(
            "sgdb-key".to_string(),
            server.uri(),
            steam_entry(440, "Team Fortress 2"),
            AssetKind::Hero,
            0,
        )
        .await
        .unwrap();
        let state = state_with_sgdb(None, found);

        let results = state.sgdb_results().expect("SteamGridDB results");
        assert_eq!(results.candidates().len(), 6);
        assert_eq!(page_ids(&state), [1, 2, 3, 4]);
        assert_eq!(results.counter(), "1 to 4 of 6");
        assert!(!results.has_previous());
        assert!(results.has_next());
        assert!(!state.busy());
    }

    #[test]
    fn next_and_previous_move_by_four() {
        let mut state = state_with_sgdb(None, sgdb_candidates("http://cdn", 1..=9));
        send(&mut state, Message::SgdbPrevious);
        assert_eq!(page_ids(&state), [1, 2, 3, 4]);

        send(&mut state, Message::SgdbNext);
        assert_eq!(page_ids(&state), [5, 6, 7, 8]);
        assert_eq!(state.sgdb_results().unwrap().counter(), "5 to 8 of 9");
        assert!(state.sgdb_results().unwrap().has_previous());

        send(&mut state, Message::SgdbNext);
        assert_eq!(page_ids(&state), [9]);
        assert_eq!(state.sgdb_results().unwrap().counter(), "9 of 9");
        assert!(!state.sgdb_results().unwrap().has_next());

        // Next at the list edge does nothing.
        send(&mut state, Message::SgdbNext);
        assert_eq!(page_ids(&state), [9]);

        send(&mut state, Message::SgdbPrevious);
        assert_eq!(page_ids(&state), [5, 6, 7, 8]);
    }

    #[tokio::test]
    async fn thumbnails_load_for_the_current_page_only() {
        let server = MockServer::start().await;
        for id in 1..=6 {
            Mock::given(method("GET"))
                .and(path(format!("/thumb/{id}.png")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(png_bytes(4, 6)))
                .expect(if id <= 4 { 1 } else { 0 })
                .mount(&server)
                .await;
        }
        let mut state = state_with_sgdb(None, sgdb_candidates(&server.uri(), 1..=6));
        let thumbs =
            load_thumbs(server.uri(), state.sgdb_results().unwrap().thumbs_to_load()).await;
        assert_eq!(thumbs.len(), 4);
        send(
            &mut state,
            Message::SgdbThumbsLoaded {
                appid: 440,
                kind: AssetKind::Hero,
                thumbs,
            },
        );
        let results = state.sgdb_results().unwrap();
        assert!(matches!(results.thumb(1), Thumb::Ready(_)));
        assert!(matches!(results.thumb(5), Thumb::Loading));
        // The loaded thumbnails are not loaded again.
        assert!(results.thumbs_to_load().is_empty());
    }

    #[test]
    fn a_failed_thumbnail_shows_as_failed() {
        let mut state = state_with_sgdb(None, sgdb_candidates("http://cdn", 1..=2));
        send(
            &mut state,
            Message::SgdbThumbsLoaded {
                appid: 440,
                kind: AssetKind::Hero,
                thumbs: vec![(1, None), (2, Some(pixel()))],
            },
        );
        let results = state.sgdb_results().unwrap();
        assert!(matches!(results.thumb(1), Thumb::Failed));
        assert!(matches!(results.thumb(2), Thumb::Ready(_)));
    }

    #[test]
    fn thumbnails_for_another_slot_are_dropped() {
        let mut state = state_with_sgdb(None, sgdb_candidates("http://cdn", 1..=2));
        send(
            &mut state,
            Message::SgdbThumbsLoaded {
                appid: 440,
                kind: AssetKind::Logo,
                thumbs: vec![(1, Some(pixel()))],
            },
        );
        assert!(matches!(
            state.sgdb_results().unwrap().thumb(1),
            Thumb::Loading
        ));
    }

    #[tokio::test]
    async fn apply_downloads_only_the_chosen_full_image() {
        let server = MockServer::start().await;
        for id in 1..=6 {
            Mock::given(method("GET"))
                .and(path(format!("/full/{id}.png")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(png_bytes(1920, 640)))
                .expect(if id == 6 { 1 } else { 0 })
                .mount(&server)
                .await;
        }
        let temp = TempDir::new().unwrap();
        let mut state = state_with_sgdb(
            Some(temp.path().to_path_buf()),
            sgdb_candidates(&server.uri(), 1..=6),
        );
        send(&mut state, Message::SgdbNext);
        send(&mut state, Message::ApplyPressed(5));
        assert!(state.busy());

        let chosen = state.sgdb_results().unwrap().candidates()[5].clone();
        let result = download_sgdb(server.uri(), chosen, AssetKind::Hero).await;
        send(
            &mut state,
            Message::SgdbDownloaded {
                appid: 440,
                kind: AssetKind::Hero,
                result,
            },
        );

        assert!(!state.busy());
        let written = temp
            .path()
            .join("userdata")
            .join(ACCOUNT_ID.to_string())
            .join("config/grid/440_hero.png");
        let image = image::open(&written).unwrap();
        assert_eq!((image.width(), image.height()), (1920, 620));
        assert!(matches!(
            state.slot_art(AssetKind::Hero),
            SlotArt::Custom(_)
        ));
        assert!(!state.status().unwrap().is_error());
    }

    #[test]
    fn a_failed_download_keeps_the_page() {
        let temp = TempDir::new().unwrap();
        let mut state = state_with_sgdb(
            Some(temp.path().to_path_buf()),
            sgdb_candidates("http://cdn", 1..=6),
        );
        send(&mut state, Message::SgdbNext);
        send(&mut state, Message::ApplyPressed(4));
        send(
            &mut state,
            Message::SgdbDownloaded {
                appid: 440,
                kind: AssetKind::Hero,
                result: Err("SteamGridDB returned HTTP 404".to_string()),
            },
        );
        assert!(!state.busy());
        let status = state.status().expect("a status line");
        assert!(status.is_error(), "{}", status.text());
        assert!(status.text().contains("HTTP 404"), "{}", status.text());
        assert_eq!(page_ids(&state), [5, 6]);
    }

    #[test]
    fn apply_while_busy_does_nothing() {
        let mut state = state_with_sgdb(None, sgdb_candidates("http://cdn", 1..=2));
        send(&mut state, Message::ApplyPressed(0));
        assert!(state.busy());
        send(&mut state, Message::ApplyPressed(1));
        // A second press while the first download runs changes nothing.
        assert!(state.busy());
        assert!(state.status().is_none_or(|status| !status.is_error()));
    }

    #[test]
    fn a_new_search_or_target_or_entry_resets_to_page_one() {
        let mut state = state_with_sgdb(None, sgdb_candidates("http://cdn", 1..=9));
        send(&mut state, Message::SgdbNext);
        send(&mut state, Message::ActionPressed);
        assert!(state.sgdb_results().is_none());
        send(
            &mut state,
            listed(
                440,
                AssetKind::Hero,
                0,
                sgdb_candidates("http://cdn", 1..=9),
            ),
        );
        assert_eq!(page_ids(&state), [1, 2, 3, 4]);

        send(&mut state, Message::SgdbNext);
        send(&mut state, Message::TargetSelected(AssetKind::Logo));
        assert!(state.sgdb_results().is_none());

        send(&mut state, Message::TargetSelected(AssetKind::Hero));
        send(&mut state, Message::ActionPressed);
        send(
            &mut state,
            listed(
                440,
                AssetKind::Hero,
                0,
                sgdb_candidates("http://cdn", 1..=9),
            ),
        );
        send(&mut state, Message::SgdbNext);
        send(&mut state, Message::EntrySelected(570));
        assert!(state.sgdb_results().is_none());
    }

    #[test]
    fn a_list_reply_for_another_entry_is_dropped() {
        let mut state = state_with(
            services(&[(Service::SteamGridDb, "sgdb-key")]),
            None,
            vec![
                steam_entry(440, "Team Fortress 2"),
                steam_entry(570, "Dota 2"),
            ],
        );
        send(&mut state, Message::EntrySelected(440));
        pick(&mut state, AssetKind::Hero, PanelTab::SteamGridDb);
        send(&mut state, Message::ActionPressed);
        send(&mut state, Message::EntrySelected(570));
        send(
            &mut state,
            listed(
                440,
                AssetKind::Hero,
                0,
                sgdb_candidates("http://cdn", 1..=2),
            ),
        );
        assert!(!state.busy());
        assert!(state.sgdb_results().is_none());
    }

    #[test]
    fn an_empty_search_shows_a_message() {
        let state = state_with_sgdb(None, Vec::new());
        assert!(state.sgdb_results().is_none());
        let status = state.status().expect("a status line");
        assert!(status.text().contains("No candidates"), "{}", status.text());
    }

    #[test]
    fn a_failed_search_shows_the_error() {
        let mut state = state_with_sgdb(None, Vec::new());
        send(&mut state, Message::ActionPressed);
        send(
            &mut state,
            Message::SgdbListed {
                appid: 440,
                kind: AssetKind::Hero,
                api_page: 0,
                result: Err("SteamGridDB returned HTTP 500".to_string()),
            },
        );
        assert!(!state.busy());
        assert_eq!(
            state.status().map(Status::text),
            Some("SteamGridDB returned HTTP 500")
        );
    }

    #[test]
    fn next_past_a_full_api_page_fetches_the_next_api_page() {
        let full = crate::sgdb::SGDB_PAGE_SIZE as u64;
        let mut state = state_with_sgdb(None, sgdb_candidates("http://cdn", 1..=full));
        let results = state.sgdb_results().unwrap();
        assert_eq!(results.counter(), format!("1 to 4 of {full}+"));
        // Go to the page before the last loaded page (items 45 to 48).
        for _ in 0..11 {
            send(&mut state, Message::SgdbNext);
        }
        assert_eq!(page_ids(&state), [45, 46, 47, 48]);
        assert!(!state.busy());

        // The next page needs items the app has not loaded yet.
        send(&mut state, Message::SgdbNext);
        assert!(state.busy());
        assert_eq!(page_ids(&state), [45, 46, 47, 48]);

        send(
            &mut state,
            listed(
                440,
                AssetKind::Hero,
                1,
                sgdb_candidates("http://cdn", full + 1..=full + 3),
            ),
        );
        assert!(!state.busy());
        let results = state.sgdb_results().unwrap();
        assert_eq!(results.candidates().len(), 53);
        assert_eq!(page_ids(&state), [49, 50, 51, 52]);
        assert_eq!(results.counter(), "49 to 52 of 53");

        send(&mut state, Message::SgdbNext);
        assert!(!state.busy());
        assert_eq!(page_ids(&state), [53]);
    }

    #[test]
    fn a_failed_api_page_ends_the_list_and_keeps_the_loaded_images() {
        let full = crate::sgdb::SGDB_PAGE_SIZE as u64;
        let mut state = state_with_sgdb(None, sgdb_candidates("http://cdn", 1..=full));
        for _ in 0..12 {
            send(&mut state, Message::SgdbNext);
        }
        send(
            &mut state,
            Message::SgdbListed {
                appid: 440,
                kind: AssetKind::Hero,
                api_page: 1,
                result: Err("SteamGridDB returned HTTP 500".to_string()),
            },
        );
        assert!(state.status().unwrap().is_error());
        assert_eq!(page_ids(&state), [45, 46, 47, 48]);
        // Next now shows the last loaded images and asks for nothing.
        send(&mut state, Message::SgdbNext);
        assert!(!state.busy());
        assert_eq!(page_ids(&state), [49, 50]);
    }

    #[test]
    fn an_api_page_with_no_new_images_ends_the_list() {
        let full = crate::sgdb::SGDB_PAGE_SIZE as u64;
        let mut state = state_with_sgdb(None, sgdb_candidates("http://cdn", 1..=full));
        for _ in 0..12 {
            send(&mut state, Message::SgdbNext);
        }
        // The API ignored the page number and sent page 0 again.
        send(
            &mut state,
            listed(
                440,
                AssetKind::Hero,
                1,
                sgdb_candidates("http://cdn", 1..=full),
            ),
        );
        let results = state.sgdb_results().unwrap();
        assert_eq!(results.candidates().len(), 50);
        assert_eq!(page_ids(&state), [49, 50]);
        assert!(!results.has_next());
        assert_eq!(results.counter(), "49 to 50 of 50");
    }

    fn pixel() -> Handle {
        Handle::from_rgba(1, 1, vec![0, 0, 0, 255])
    }

    /// A state with Team Fortress 2 selected and art in the given slots.
    fn state_with_art(steam_root: Option<PathBuf>, art: Vec<(AssetKind, SlotArt)>) -> State {
        let mut state = state_with(
            services(&[(Service::OpenAi, "sk-test")]),
            steam_root,
            vec![steam_entry(440, "Team Fortress 2")],
        );
        send(&mut state, Message::EntrySelected(440));
        send(&mut state, Message::SlotArtLoaded { appid: 440, art });
        state
    }

    #[test]
    fn selecting_an_entry_marks_every_slot_as_loading() {
        let mut state = state_with(
            services(&[]),
            Some(PathBuf::from("/steam")),
            vec![steam_entry(440, "Team Fortress 2")],
        );
        for kind in AssetKind::ALL {
            assert!(matches!(state.slot_art(kind), SlotArt::Empty), "{kind:?}");
        }
        send(&mut state, Message::EntrySelected(440));
        for kind in AssetKind::ALL {
            assert!(matches!(state.slot_art(kind), SlotArt::Loading), "{kind:?}");
        }
    }

    #[test]
    fn slot_art_reply_sets_the_slot_images() {
        let state = state_with_art(
            Some(PathBuf::from("/steam")),
            vec![
                (AssetKind::Hero, SlotArt::Custom(pixel())),
                (AssetKind::Portrait, SlotArt::Default(pixel())),
                (AssetKind::Wide, SlotArt::Empty),
                (AssetKind::Logo, SlotArt::Empty),
            ],
        );
        assert!(matches!(
            state.slot_art(AssetKind::Hero),
            SlotArt::Custom(_)
        ));
        assert!(matches!(
            state.slot_art(AssetKind::Portrait),
            SlotArt::Default(_)
        ));
        assert!(matches!(state.slot_art(AssetKind::Wide), SlotArt::Empty));
    }

    #[test]
    fn slot_art_reply_for_another_game_is_dropped() {
        let mut state = state_with(
            services(&[]),
            Some(PathBuf::from("/steam")),
            vec![
                steam_entry(440, "Team Fortress 2"),
                steam_entry(570, "Dota 2"),
            ],
        );
        send(&mut state, Message::EntrySelected(570));
        send(
            &mut state,
            Message::SlotArtLoaded {
                appid: 440,
                art: vec![(AssetKind::Hero, SlotArt::Custom(pixel()))],
            },
        );
        assert!(matches!(state.slot_art(AssetKind::Hero), SlotArt::Loading));
    }

    #[test]
    fn apply_shows_the_written_art_in_the_slot() {
        let temp = TempDir::new().unwrap();
        let mut state = state_with_art(
            Some(temp.path().to_path_buf()),
            vec![(AssetKind::Hero, SlotArt::Empty)],
        );
        pick(&mut state, AssetKind::Hero, PanelTab::SteamGridDb);
        send(
            &mut state,
            candidates(440, AssetKind::Hero, vec![solid(1920, 620)]),
        );
        send(&mut state, Message::ApplyPressed(0));
        assert!(matches!(
            state.slot_art(AssetKind::Hero),
            SlotArt::Custom(_)
        ));
    }

    #[test]
    fn arrived_candidates_have_image_handles() {
        let mut state = state_with_art(Some(PathBuf::from("/steam")), vec![]);
        send(
            &mut state,
            candidates(440, AssetKind::Portrait, vec![solid(6, 9), solid(6, 9)]),
        );
        assert_eq!(state.candidate_handles().len(), 2);
        send(&mut state, Message::EntrySelected(440));
        assert!(state.candidate_handles().is_empty());
    }

    #[test]
    fn target_dropdown_sets_the_target_and_keeps_the_tab() {
        let mut state = state_with_art(Some(PathBuf::from("/steam")), vec![]);
        send(&mut state, Message::TabSelected(PanelTab::Generate));
        send(
            &mut state,
            candidates(440, AssetKind::Portrait, vec![solid(6, 9)]),
        );
        send(&mut state, Message::TargetSelected(AssetKind::Logo));
        assert_eq!(state.target(), AssetKind::Logo);
        assert_eq!(state.tab(), PanelTab::Generate);
        assert!(state.candidates().is_empty());
    }

    #[test]
    fn modify_without_current_art_shows_an_error() {
        let mut state = state_with_art(
            Some(PathBuf::from("/steam")),
            vec![(AssetKind::Portrait, SlotArt::Empty)],
        );
        pick(&mut state, AssetKind::Portrait, PanelTab::Modify);
        assert!(!state.can_modify());
        send(&mut state, Message::ActionPressed);
        assert!(!state.busy());
        let status = state.status().expect("a status line");
        assert!(status.is_error());
        assert!(
            status.text().contains("no current art"),
            "{}",
            status.text()
        );
    }

    #[test]
    fn modify_is_possible_with_default_art() {
        let state = state_with_art(
            Some(PathBuf::from("/steam")),
            vec![(AssetKind::Portrait, SlotArt::Default(pixel()))],
        );
        assert!(state.can_modify());
    }

    #[test]
    fn codex_generate_needs_no_openai_key() {
        let mut state = state_with(
            services(&[]),
            Some(PathBuf::from("/nonexistent-steam")),
            vec![steam_entry(440, "Team Fortress 2")],
        );
        state.config.image_source = ImageSource::Codex;
        send(&mut state, Message::EntrySelected(440));
        pick(&mut state, AssetKind::Hero, PanelTab::Generate);
        send(&mut state, Message::ActionPressed);

        assert!(state.busy());
        let status = state.status().expect("a status line");
        assert!(!status.is_error(), "{}", status.text());
        assert!(status.text().contains("Codex"), "{}", status.text());
    }

    #[test]
    fn codex_status_names_the_image_count() {
        for (count, expected) in [(1, "Codex makes 1 image."), (3, "Codex makes 3 images.")] {
            let mut state = state_with(
                services(&[]),
                Some(PathBuf::from("/nonexistent-steam")),
                vec![steam_entry(440, "Team Fortress 2")],
            );
            state.config.image_source = ImageSource::Codex;
            state.config.images_per_request = count;
            send(&mut state, Message::EntrySelected(440));
            pick(&mut state, AssetKind::Hero, PanelTab::Generate);
            send(&mut state, Message::ActionPressed);
            let text = state.status().expect("a status line").text();
            assert!(text.starts_with(expected), "{text}");
        }
    }

    #[test]
    fn codex_modify_needs_no_openai_key() {
        let mut state = state_with_art(
            Some(PathBuf::from("/steam")),
            vec![(AssetKind::Portrait, SlotArt::Default(pixel()))],
        );
        state.config.image_source = ImageSource::Codex;
        pick(&mut state, AssetKind::Portrait, PanelTab::Modify);
        send(&mut state, Message::ActionPressed);
        assert!(state.busy());
        assert!(!state.status().is_some_and(Status::is_error));
    }

    #[tokio::test]
    async fn codex_generate_returns_prepared_images() {
        let home = TempDir::new().unwrap();
        let codex = CodexClient::new(
            Arc::new(FakeCodex {
                home: home.path().to_path_buf(),
                size: (2172, 724),
                runs: Default::default(),
            }),
            Some(home.path().to_path_buf()),
        );
        let images = request_generate(
            ImageBackend::Codex(codex),
            "castle".into(),
            AssetKind::Hero,
            2,
        )
        .await
        .unwrap();
        assert_eq!(images.len(), 2);
        assert_eq!((images[0].width(), images[0].height()), (1920, 620));
    }

    #[tokio::test]
    async fn codex_failure_becomes_the_status_text() {
        let codex = CodexClient::new(Arc::new(NoCodex), None);
        let err = request_generate(ImageBackend::Codex(codex), "x".into(), AssetKind::Logo, 1)
            .await
            .unwrap_err();
        assert!(err.contains("Codex is not installed"), "{err}");
    }

    #[test]
    fn settings_open_checks_for_codex() {
        let mut state = State::new(services(&[]));
        send(&mut state, Message::SettingsOpened);
        assert_eq!(state.settings().unwrap().codex, CodexStatus::Checking);
        send(
            &mut state,
            Message::CodexChecked(CodexStatus::Found("codex-cli 0.167.0".to_string())),
        );
        assert_eq!(
            state.settings().unwrap().codex,
            CodexStatus::Found("codex-cli 0.167.0".to_string())
        );
    }

    #[test]
    fn late_codex_check_without_a_dialog_is_dropped() {
        let mut state = State::new(services(&[]));
        send(&mut state, Message::CodexChecked(CodexStatus::NotFound));
        assert!(state.settings().is_none());
    }

    #[tokio::test]
    async fn codex_check_reads_the_runner() {
        assert_eq!(check_codex(Arc::new(NoCodex)).await, CodexStatus::NotFound);
    }

    #[test]
    fn settings_save_writes_the_image_source() {
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join("config.toml");
        let mut services = services(&[]);
        services.config_path = Some(config_path.clone());
        let mut state = State::new(services);

        send(&mut state, Message::SettingsOpened);
        send(&mut state, Message::ImageSourceSelected(ImageSource::Codex));
        assert_eq!(state.settings().unwrap().image_source, ImageSource::Codex);
        send(&mut state, Message::SettingsSaved);
        assert_eq!(
            Config::load(&config_path).unwrap().image_source,
            ImageSource::Codex
        );
    }

    #[test]
    fn settings_save_writes_the_images_per_request() {
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join("config.toml");
        let mut services = services(&[]);
        services.config_path = Some(config_path.clone());
        let mut state = State::new(services);

        send(&mut state, Message::SettingsOpened);
        send(&mut state, Message::ImagesPerRequestSelected(3));
        assert_eq!(state.settings().unwrap().images_per_request, 3);
        send(&mut state, Message::SettingsSaved);
        assert_eq!(Config::load(&config_path).unwrap().images_per_request, 3);
    }

    #[test]
    fn prompt_editor_updates_the_prompt() {
        use iced::widget::text_editor::{Action, Edit};

        let mut state = State::new(services(&[]));
        send(
            &mut state,
            Message::PromptEdited(Action::Edit(Edit::Insert('a'))),
        );
        send(
            &mut state,
            Message::PromptEdited(Action::Edit(Edit::Insert('b'))),
        );
        assert_eq!(state.prompt(), "ab");

        send(&mut state, Message::PromptChanged("knight".to_string()));
        assert_eq!(state.prompt(), "knight");
        assert_eq!(state.prompt_editor().text(), "knight");
    }

    #[test]
    fn settings_open_with_the_current_values() {
        let mut state = State::new(services(&[(Service::SteamGridDb, "grid-key")]));
        assert!(state.settings().is_none());
        send(&mut state, Message::SettingsOpened);
        let form = state.settings().expect("an open dialog");
        assert_eq!(
            form.value(SettingsField::Key(Service::SteamGridDb)),
            "grid-key"
        );
        assert_eq!(
            form.value(SettingsField::Model),
            crate::config::DEFAULT_OPENAI_MODEL
        );
    }

    #[test]
    fn settings_close_discards_the_changes() {
        let mut state = State::new(services(&[]));
        send(&mut state, Message::SettingsOpened);
        send(
            &mut state,
            Message::SettingsChanged(SettingsField::Key(Service::OpenAi), "sk-new".to_string()),
        );
        send(&mut state, Message::SettingsClosed);
        assert!(state.settings().is_none());
        assert_eq!(state.services.secrets.get_key(Service::OpenAi), None);
    }

    #[test]
    fn settings_save_writes_the_keys_and_the_config() {
        let temp = TempDir::new().unwrap();
        let config_path = temp.path().join("config.toml");
        let mut services = services(&[(Service::SteamWeb, "old-steam")]);
        services.config_path = Some(config_path.clone());
        let mut state = State::new(services);

        send(&mut state, Message::SettingsOpened);
        send(
            &mut state,
            Message::SettingsChanged(SettingsField::Key(Service::OpenAi), "sk-new".to_string()),
        );
        send(
            &mut state,
            Message::SettingsChanged(SettingsField::Key(Service::SteamWeb), String::new()),
        );
        send(
            &mut state,
            Message::SettingsChanged(SettingsField::SteamRoot, "/games/Steam".to_string()),
        );
        send(
            &mut state,
            Message::SettingsChanged(SettingsField::Model, "gpt-image-2.6-flare".to_string()),
        );
        send(&mut state, Message::SettingsSaved);

        assert!(state.settings().is_none());
        let secrets = &state.services.secrets;
        assert_eq!(secrets.get_key(Service::OpenAi).as_deref(), Some("sk-new"));
        assert_eq!(secrets.get_key(Service::SteamWeb), None);
        assert_eq!(
            Config::load(&config_path).unwrap(),
            Config {
                steam_root: Some(PathBuf::from("/games/Steam")),
                openai_model: "gpt-image-2.6-flare".to_string(),
                ..Config::default()
            }
        );
        // The library loads again with the saved settings.
        assert!(state.busy());
    }

    #[test]
    fn settings_save_failure_keeps_the_dialog_open() {
        let temp = TempDir::new().unwrap();
        let mut services = services(&[]);
        // A folder cannot be written as a file.
        services.config_path = Some(temp.path().to_path_buf());
        let mut state = State::new(services);

        send(&mut state, Message::SettingsOpened);
        send(
            &mut state,
            Message::SettingsChanged(SettingsField::Model, "gpt-image-2".to_string()),
        );
        send(&mut state, Message::SettingsSaved);

        let form = state.settings().expect("the dialog stays open");
        assert_eq!(form.value(SettingsField::Model), "gpt-image-2");
        let error = form.error.as_deref().expect("a save error");
        assert!(error.contains("Cannot save the settings"), "{error}");
        assert!(!state.busy());
    }

    #[test]
    fn settings_save_waits_for_a_running_request() {
        let mut state = State::new(services(&[]));
        send(&mut state, Message::SettingsOpened);
        state.busy = true;
        send(&mut state, Message::SettingsSaved);
        assert!(state.settings().is_some());
    }

    fn slot_input(grid_dir: Option<PathBuf>, cdn_base: &str, entry: LibraryEntry) -> SlotArtInput {
        SlotArtInput {
            grid_dir,
            cache_dir: fixture_root().join("appcache/librarycache"),
            cdn_base: cdn_base.to_string(),
            entry,
        }
    }

    #[tokio::test]
    async fn slot_art_prefers_the_custom_file() {
        let grid = TempDir::new().unwrap();
        fs::write(grid.path().join("440_hero.png"), png_bytes(4, 4)).unwrap();
        let input = slot_input(
            Some(grid.path().to_path_buf()),
            "http://127.0.0.1:1",
            steam_entry(440, "Team Fortress 2"),
        );
        let art = load_slot_art(&input, AssetKind::Hero).await;
        assert!(matches!(art, SlotArt::Custom(_)), "{art:?}");
    }

    #[tokio::test]
    async fn slot_art_falls_back_to_the_library_cache() {
        let grid = TempDir::new().unwrap();
        let input = slot_input(
            Some(grid.path().to_path_buf()),
            "http://127.0.0.1:1",
            steam_entry(440, "Team Fortress 2"),
        );
        let art = load_slot_art(&input, AssetKind::Hero).await;
        assert!(matches!(art, SlotArt::Default(_)), "{art:?}");
    }

    #[tokio::test]
    async fn slot_art_without_a_grid_folder_still_shows_default_art() {
        let input = slot_input(
            None,
            "http://127.0.0.1:1",
            steam_entry(440, "Team Fortress 2"),
        );
        let art = load_slot_art(&input, AssetKind::Portrait).await;
        assert!(matches!(art, SlotArt::Default(_)), "{art:?}");
    }

    #[tokio::test]
    async fn slot_art_falls_back_to_the_cdn() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/570/header.jpg"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(png_bytes(4, 4)))
            .expect(1)
            .mount(&server)
            .await;
        let input = slot_input(None, &server.uri(), steam_entry(570, "Dota 2"));
        let art = load_slot_art(&input, AssetKind::Wide).await;
        assert!(matches!(art, SlotArt::Default(_)), "{art:?}");
    }

    #[tokio::test]
    async fn slot_art_is_empty_when_the_cdn_has_no_file() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/570/header.jpg"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;
        let input = slot_input(None, &server.uri(), steam_entry(570, "Dota 2"));
        let art = load_slot_art(&input, AssetKind::Wide).await;
        assert!(matches!(art, SlotArt::Empty), "{art:?}");
    }

    #[tokio::test]
    async fn slot_art_is_empty_for_a_shortcut_without_custom_art() {
        let mut entry = steam_entry(3_000_000_000, "My Game");
        entry.source = EntrySource::NonSteam;
        let input = slot_input(None, "http://127.0.0.1:1", entry);
        let art = load_slot_art(&input, AssetKind::Hero).await;
        assert!(matches!(art, SlotArt::Empty), "{art:?}");
    }
}
