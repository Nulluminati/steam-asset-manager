//! Every OS-specific Steam path lives in this module.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    MacOs,
    Windows,
}

/// The parts of the environment that root discovery reads. Tests use a fake.
pub trait PathEnv {
    fn os(&self) -> Os;
    fn home_dir(&self) -> Option<PathBuf>;
    /// `HKCU\Software\Valve\Steam\SteamPath`. Always `None` off Windows.
    fn registry_steam_path(&self) -> Option<PathBuf>;
    /// The `ProgramFiles(x86)` environment variable. Only used on Windows.
    fn program_files_x86(&self) -> Option<PathBuf>;
}

/// The real environment of this process.
pub struct SystemEnv;

impl PathEnv for SystemEnv {
    fn os(&self) -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Linux
        }
    }

    fn home_dir(&self) -> Option<PathBuf> {
        directories::BaseDirs::new().map(|dirs| dirs.home_dir().to_path_buf())
    }

    #[cfg(windows)]
    fn registry_steam_path(&self) -> Option<PathBuf> {
        use winreg::RegKey;
        use winreg::enums::HKEY_CURRENT_USER;

        let key = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Valve\Steam")
            .ok()?;
        let path: String = key.get_value("SteamPath").ok()?;
        Some(PathBuf::from(path))
    }

    #[cfg(not(windows))]
    fn registry_steam_path(&self) -> Option<PathBuf> {
        None
    }

    fn program_files_x86(&self) -> Option<PathBuf> {
        std::env::var_os("ProgramFiles(x86)").map(PathBuf::from)
    }
}

/// The folders that can hold the Steam root, in the order to try them.
pub fn root_candidates(env: &dyn PathEnv) -> Vec<PathBuf> {
    match env.os() {
        Os::Linux => match env.home_dir() {
            Some(home) => vec![
                home.join(".local/share/Steam"),
                home.join(".steam/steam"),
                home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
            ],
            None => Vec::new(),
        },
        Os::MacOs => match env.home_dir() {
            Some(home) => vec![home.join("Library/Application Support/Steam")],
            None => Vec::new(),
        },
        Os::Windows => {
            let program_files = env
                .program_files_x86()
                .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)"));
            env.registry_steam_path()
                .into_iter()
                .chain([program_files.join("Steam")])
                .collect()
        }
    }
}

/// Finds the Steam root folder.
///
/// An override path wins when it is a folder. Otherwise the first
/// candidate from [`root_candidates`] that is a folder wins.
pub fn find_steam_root(env: &dyn PathEnv, override_path: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = override_path.filter(|path| path.is_dir()) {
        return Some(path.to_path_buf());
    }
    root_candidates(env).into_iter().find(|path| path.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    struct FakeEnv {
        os: Os,
        home: Option<PathBuf>,
        registry: Option<PathBuf>,
        program_files_x86: Option<PathBuf>,
    }

    impl FakeEnv {
        fn new(os: Os, home: &Path) -> Self {
            FakeEnv {
                os,
                home: Some(home.to_path_buf()),
                registry: None,
                program_files_x86: None,
            }
        }
    }

    impl PathEnv for FakeEnv {
        fn os(&self) -> Os {
            self.os
        }
        fn home_dir(&self) -> Option<PathBuf> {
            self.home.clone()
        }
        fn registry_steam_path(&self) -> Option<PathBuf> {
            self.registry.clone()
        }
        fn program_files_x86(&self) -> Option<PathBuf> {
            self.program_files_x86.clone()
        }
    }

    fn make_dir(base: &Path, rel: &str) -> PathBuf {
        let dir = base.join(rel);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn linux_candidate_order() {
        let home = PathBuf::from("/home/zero");
        let env = FakeEnv::new(Os::Linux, &home);
        assert_eq!(
            root_candidates(&env),
            vec![
                home.join(".local/share/Steam"),
                home.join(".steam/steam"),
                home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
            ]
        );
    }

    #[test]
    fn linux_prefers_local_share() {
        let home = TempDir::new().unwrap();
        let local = make_dir(home.path(), ".local/share/Steam");
        make_dir(home.path(), ".steam/steam");
        make_dir(
            home.path(),
            ".var/app/com.valvesoftware.Steam/.local/share/Steam",
        );
        let env = FakeEnv::new(Os::Linux, home.path());
        assert_eq!(find_steam_root(&env, None), Some(local));
    }

    #[test]
    fn linux_falls_back_to_dot_steam() {
        let home = TempDir::new().unwrap();
        let dot_steam = make_dir(home.path(), ".steam/steam");
        make_dir(
            home.path(),
            ".var/app/com.valvesoftware.Steam/.local/share/Steam",
        );
        let env = FakeEnv::new(Os::Linux, home.path());
        assert_eq!(find_steam_root(&env, None), Some(dot_steam));
    }

    #[test]
    fn linux_falls_back_to_flatpak() {
        let home = TempDir::new().unwrap();
        let flatpak = make_dir(
            home.path(),
            ".var/app/com.valvesoftware.Steam/.local/share/Steam",
        );
        let env = FakeEnv::new(Os::Linux, home.path());
        assert_eq!(find_steam_root(&env, None), Some(flatpak));
    }

    #[test]
    fn linux_without_steam_finds_nothing() {
        let home = TempDir::new().unwrap();
        let env = FakeEnv::new(Os::Linux, home.path());
        assert_eq!(find_steam_root(&env, None), None);
    }

    #[test]
    fn linux_ignores_a_file_in_place_of_the_folder() {
        let home = TempDir::new().unwrap();
        make_dir(home.path(), ".local/share");
        fs::write(home.path().join(".local/share/Steam"), b"").unwrap();
        let env = FakeEnv::new(Os::Linux, home.path());
        assert_eq!(find_steam_root(&env, None), None);
    }

    #[test]
    fn no_home_gives_no_linux_candidates() {
        let env = FakeEnv {
            os: Os::Linux,
            home: None,
            registry: None,
            program_files_x86: None,
        };
        assert!(root_candidates(&env).is_empty());
    }

    #[test]
    fn macos_candidate_order() {
        let home = PathBuf::from("/Users/zero");
        let env = FakeEnv::new(Os::MacOs, &home);
        assert_eq!(
            root_candidates(&env),
            vec![home.join("Library/Application Support/Steam")]
        );
    }

    #[test]
    fn macos_finds_application_support() {
        let home = TempDir::new().unwrap();
        let root = make_dir(home.path(), "Library/Application Support/Steam");
        let env = FakeEnv::new(Os::MacOs, home.path());
        assert_eq!(find_steam_root(&env, None), Some(root));
    }

    #[test]
    fn windows_candidate_order() {
        let env = FakeEnv {
            os: Os::Windows,
            home: None,
            registry: Some(PathBuf::from("D:/Games/Steam")),
            program_files_x86: Some(PathBuf::from("E:/Program Files (x86)")),
        };
        assert_eq!(
            root_candidates(&env),
            vec![
                PathBuf::from("D:/Games/Steam"),
                PathBuf::from("E:/Program Files (x86)").join("Steam"),
            ]
        );
    }

    #[test]
    fn windows_default_program_files_folder() {
        let env = FakeEnv {
            os: Os::Windows,
            home: None,
            registry: None,
            program_files_x86: None,
        };
        assert_eq!(
            root_candidates(&env),
            vec![PathBuf::from(r"C:\Program Files (x86)").join("Steam")]
        );
    }

    #[test]
    fn windows_prefers_registry() {
        let base = TempDir::new().unwrap();
        let registry_root = make_dir(base.path(), "registry/Steam");
        let program_files = make_dir(base.path(), "pf86");
        make_dir(&program_files, "Steam");
        let env = FakeEnv {
            os: Os::Windows,
            home: None,
            registry: Some(registry_root.clone()),
            program_files_x86: Some(program_files),
        };
        assert_eq!(find_steam_root(&env, None), Some(registry_root));
    }

    #[test]
    fn windows_falls_back_to_program_files() {
        let base = TempDir::new().unwrap();
        let program_files = make_dir(base.path(), "pf86");
        let steam = make_dir(&program_files, "Steam");
        let env = FakeEnv {
            os: Os::Windows,
            home: None,
            registry: Some(base.path().join("registry/missing")),
            program_files_x86: Some(program_files),
        };
        assert_eq!(find_steam_root(&env, None), Some(steam));
    }

    #[test]
    fn override_wins_over_candidates() {
        let home = TempDir::new().unwrap();
        make_dir(home.path(), ".local/share/Steam");
        let custom = make_dir(home.path(), "custom/Steam");
        let env = FakeEnv::new(Os::Linux, home.path());
        assert_eq!(find_steam_root(&env, Some(&custom)), Some(custom));
    }

    #[test]
    fn missing_override_falls_back_to_candidates() {
        let home = TempDir::new().unwrap();
        let local = make_dir(home.path(), ".local/share/Steam");
        let env = FakeEnv::new(Os::Linux, home.path());
        let missing = home.path().join("missing");
        assert_eq!(find_steam_root(&env, Some(&missing)), Some(local));
    }
}
