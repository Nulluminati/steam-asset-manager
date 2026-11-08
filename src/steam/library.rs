use std::collections::HashSet;
use std::path::{Path, PathBuf};

use keyvalues_parser::Value;

use super::{SteamError, get_str, parse_vdf, read_text};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntrySource {
    Steam,
    NonSteam,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryEntry {
    pub appid: u32,
    pub name: String,
    pub source: EntrySource,
    pub installed: bool,
}

/// Sorts entries by name without regard to case, then by appid.
pub fn sort_entries(entries: &mut [LibraryEntry]) {
    entries.sort_by_cached_key(|entry| (entry.name.to_lowercase(), entry.appid));
}

/// Parses the text of `libraryfolders.vdf` and returns the library folders
/// in file order. Supports the current format (`"0" { "path" "..." }`) and
/// the old format (`"1" "D:\\SteamLibrary"`).
pub fn parse_library_folders(text: &str) -> Result<Vec<PathBuf>, SteamError> {
    let root = parse_vdf(text, "libraryfolders.vdf")?;
    let mut numbered: Vec<(u32, PathBuf)> = Vec::new();
    for (key, values) in root.iter() {
        let Ok(index) = key.parse::<u32>() else {
            continue;
        };
        let path = match values.first() {
            Some(Value::Obj(folder)) => get_str(folder, "path"),
            Some(Value::Str(path)) => Some(path.as_ref()),
            None => None,
        };
        if let Some(path) = path {
            numbered.push((index, PathBuf::from(path)));
        }
    }
    // The parser stores keys as text, so "10" sorts before "2".
    numbered.sort_by_key(|(index, _)| *index);
    Ok(numbered.into_iter().map(|(_, path)| path).collect())
}

/// Parses the text of one `appmanifest_<appid>.acf` file.
pub fn parse_app_manifest(text: &str) -> Result<LibraryEntry, SteamError> {
    let state = parse_vdf(text, "app manifest")?;
    let parse_error = |message: &str| SteamError::Parse {
        what: "app manifest".to_string(),
        message: message.to_string(),
    };
    let appid = get_str(&state, "appid")
        .and_then(|appid| appid.parse::<u32>().ok())
        .ok_or_else(|| parse_error("no valid appid"))?;
    let name = get_str(&state, "name").ok_or_else(|| parse_error("no name"))?;
    Ok(LibraryEntry {
        appid,
        name: name.to_string(),
        source: EntrySource::Steam,
        installed: true,
    })
}

/// Reads every app manifest in the Steam root and in each folder that
/// `<root>/steamapps/libraryfolders.vdf` lists. Returns the games sorted by
/// name. Missing library folders and broken manifests are skipped.
pub fn installed_games(steam_root: &Path) -> Result<Vec<LibraryEntry>, SteamError> {
    let folders_file = steam_root.join("steamapps").join("libraryfolders.vdf");
    let mut folders = vec![steam_root.to_path_buf()];
    if folders_file.is_file() {
        folders.extend(parse_library_folders(&read_text(&folders_file)?)?);
    }

    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    for folder in folders {
        for manifest in app_manifests(&folder.join("steamapps")) {
            let Ok(text) = read_text(&manifest) else {
                continue;
            };
            let Ok(entry) = parse_app_manifest(&text) else {
                continue;
            };
            if seen.insert(entry.appid) {
                entries.push(entry);
            }
        }
    }
    sort_entries(&mut entries);
    Ok(entries)
}

/// The `appmanifest_*.acf` files in one `steamapps` folder. A folder that
/// cannot be read (for example, an unplugged drive) has none.
fn app_manifests(steamapps: &Path) -> Vec<PathBuf> {
    let Ok(dir) = std::fs::read_dir(steamapps) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = dir
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("appmanifest_") && name.ends_with(".acf"))
        })
        .collect();
    paths.sort();
    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn fixture_library(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/libraries")
            .join(name)
    }

    fn vdf_path(path: &Path) -> String {
        path.display().to_string().replace('\\', "\\\\")
    }

    fn steam(appid: u32, name: &str) -> LibraryEntry {
        LibraryEntry {
            appid,
            name: name.to_string(),
            source: EntrySource::Steam,
            installed: true,
        }
    }

    /// A temp Steam root whose `libraryfolders.vdf` lists the given folders.
    fn root_with_libraries(folders: &[PathBuf]) -> TempDir {
        let root = TempDir::new().unwrap();
        fs::create_dir_all(root.path().join("steamapps")).unwrap();
        let mut text = String::from("\"libraryfolders\"\n{\n");
        for (i, folder) in folders.iter().enumerate() {
            text.push_str(&format!(
                "\t\"{i}\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t\t\"label\"\t\t\"\"\n\t}}\n",
                vdf_path(folder)
            ));
        }
        text.push_str("}\n");
        fs::write(root.path().join("steamapps/libraryfolders.vdf"), text).unwrap();
        root
    }

    #[test]
    fn parses_current_library_folders_format() {
        let text = r#"
"libraryfolders"
{
    "0"
    {
        "path"      "C:\\Program Files (x86)\\Steam"
        "label"     ""
        "apps"
        {
            "228980"    "12345"
        }
    }
    "1"
    {
        "path"      "D:\\SteamLibrary"
    }
}
"#;
        assert_eq!(
            parse_library_folders(text).unwrap(),
            vec![
                PathBuf::from(r"C:\Program Files (x86)\Steam"),
                PathBuf::from(r"D:\SteamLibrary"),
            ]
        );
    }

    #[test]
    fn parses_old_library_folders_format() {
        let text = r#"
"LibraryFolders"
{
    "TimeNextStatsReport"   "1700000000"
    "ContentStatsID"        "-123"
    "1"     "/mnt/games/SteamLibrary"
}
"#;
        assert_eq!(
            parse_library_folders(text).unwrap(),
            vec![PathBuf::from("/mnt/games/SteamLibrary")]
        );
    }

    #[test]
    fn parses_app_manifest() {
        let text =
            fs::read_to_string(fixture_library("a").join("steamapps/appmanifest_440.acf")).unwrap();
        assert_eq!(
            parse_app_manifest(&text).unwrap(),
            steam(440, "Team Fortress 2")
        );
    }

    #[test]
    fn manifest_without_name_is_an_error() {
        let text = "\"AppState\"\n{\n\t\"appid\"\t\"999\"\n}\n";
        assert!(parse_app_manifest(text).is_err());
    }

    #[test]
    fn two_library_folders_sorted_by_name() {
        let root = root_with_libraries(&[fixture_library("a"), fixture_library("b")]);
        assert_eq!(
            installed_games(root.path()).unwrap(),
            vec![
                steam(570, "Dota 2"),
                steam(400, "portal"),
                steam(620, "Portal 2"),
                steam(440, "Team Fortress 2"),
            ]
        );
    }

    #[test]
    fn missing_library_folder_is_skipped() {
        let root = root_with_libraries(&[fixture_library("a"), fixture_library("missing")]);
        let names: Vec<_> = installed_games(root.path())
            .unwrap()
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, vec!["Dota 2", "Team Fortress 2"]);
    }

    #[test]
    fn root_is_read_without_library_folders_file() {
        let root = TempDir::new().unwrap();
        let steamapps = root.path().join("steamapps");
        fs::create_dir_all(&steamapps).unwrap();
        fs::copy(
            fixture_library("b").join("steamapps/appmanifest_620.acf"),
            steamapps.join("appmanifest_620.acf"),
        )
        .unwrap();
        assert_eq!(
            installed_games(root.path()).unwrap(),
            vec![steam(620, "Portal 2")]
        );
    }

    #[test]
    fn same_app_in_two_folders_is_listed_once() {
        let root = root_with_libraries(&[fixture_library("a"), fixture_library("a")]);
        assert_eq!(installed_games(root.path()).unwrap().len(), 2);
    }

    #[test]
    fn sort_ignores_case() {
        let mut entries = vec![steam(2, "beta"), steam(1, "Alpha"), steam(3, "Beta")];
        sort_entries(&mut entries);
        let ids: Vec<_> = entries.iter().map(|e| e.appid).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }
}
