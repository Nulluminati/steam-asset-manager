//! A read-only parser for the binary VDF file `shortcuts.vdf`.

use std::path::{Path, PathBuf};

use super::SteamError;
use super::library::{EntrySource, LibraryEntry, sort_entries};

/// The path of the shortcuts file for one account.
pub fn shortcuts_path(steam_root: &Path, account_id: u32) -> PathBuf {
    steam_root
        .join("userdata")
        .join(account_id.to_string())
        .join("config")
        .join("shortcuts.vdf")
}

/// The appid Steam gives a shortcut whose file has no `appid` field.
pub fn fallback_appid(exe: &str, app_name: &str) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(exe.as_bytes());
    hasher.update(app_name.as_bytes());
    hasher.finalize() | 0x8000_0000
}

/// Parses the bytes of `shortcuts.vdf` and returns the shortcuts sorted by
/// name, each with `EntrySource::NonSteam` and `installed: true`.
pub fn parse_shortcuts(bytes: &[u8]) -> Result<Vec<LibraryEntry>, SteamError> {
    let root = Reader { bytes, pos: 0 }.read_root()?;
    let Some(Node::Map(shortcuts)) = find(&root, "shortcuts") else {
        return Ok(Vec::new());
    };
    let mut entries: Vec<LibraryEntry> = shortcuts
        .iter()
        .filter_map(|(_, node)| match node {
            Node::Map(fields) => Some(entry_from_fields(fields)),
            _ => None,
        })
        .collect();
    sort_entries(&mut entries);
    Ok(entries)
}

/// Reads the shortcuts of one account. A missing file means no shortcuts.
pub fn load_shortcuts(steam_root: &Path, account_id: u32) -> Result<Vec<LibraryEntry>, SteamError> {
    let path = shortcuts_path(steam_root, account_id);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(SteamError::Io { path, source }),
    };
    parse_shortcuts(&bytes)
}

fn entry_from_fields(fields: &[(String, Node)]) -> LibraryEntry {
    let text = |key| match find(fields, key) {
        Some(Node::Str(value)) => value.as_str(),
        _ => "",
    };
    let name = text("AppName");
    let appid = match find(fields, "appid") {
        // Steam stores the u32 appid as an i32; keep the same bits.
        Some(Node::Int(appid)) => *appid as u32,
        _ => fallback_appid(text("Exe"), name),
    };
    LibraryEntry {
        appid,
        name: name.to_string(),
        source: EntrySource::NonSteam,
        installed: true,
    }
}

const TYPE_MAP: u8 = 0x00;
const TYPE_STRING: u8 = 0x01;
const TYPE_INT32: u8 = 0x02;
const TYPE_END: u8 = 0x08;

enum Node {
    Map(Vec<(String, Node)>),
    Str(String),
    Int(i32),
}

/// The first node of `key`, with the key matched without regard to case.
fn find<'a>(fields: &'a [(String, Node)], key: &str) -> Option<&'a Node> {
    fields
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .map(|(_, node)| node)
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    /// Reads the fields of the root map. The root map ends at an end byte
    /// or at the end of the file.
    fn read_root(&mut self) -> Result<Vec<(String, Node)>, SteamError> {
        self.read_fields(true)
    }

    fn read_fields(&mut self, is_root: bool) -> Result<Vec<(String, Node)>, SteamError> {
        let mut fields = Vec::new();
        loop {
            if is_root && self.pos == self.bytes.len() {
                return Ok(fields);
            }
            let kind = self.read_byte()?;
            if kind == TYPE_END {
                return Ok(fields);
            }
            if !matches!(kind, TYPE_MAP | TYPE_STRING | TYPE_INT32) {
                return Err(self.error(&format!("unknown type byte 0x{kind:02x}")));
            }
            let key = self.read_string()?;
            let node = match kind {
                TYPE_MAP => Node::Map(self.read_fields(false)?),
                TYPE_STRING => Node::Str(self.read_string()?),
                TYPE_INT32 => Node::Int(self.read_i32()?),
                _ => unreachable!("the type byte was checked above"),
            };
            fields.push((key, node));
        }
    }

    fn read_byte(&mut self) -> Result<u8, SteamError> {
        let byte = *self
            .bytes
            .get(self.pos)
            .ok_or_else(|| self.error("unexpected end of file"))?;
        self.pos += 1;
        Ok(byte)
    }

    fn read_string(&mut self) -> Result<String, SteamError> {
        let rest = &self.bytes[self.pos..];
        let len = rest
            .iter()
            .position(|&byte| byte == 0)
            .ok_or_else(|| self.error("unterminated string"))?;
        let text = String::from_utf8_lossy(&rest[..len]).into_owned();
        self.pos += len + 1;
        Ok(text)
    }

    fn read_i32(&mut self) -> Result<i32, SteamError> {
        let raw: [u8; 4] = self
            .bytes
            .get(self.pos..self.pos + 4)
            .and_then(|slice| slice.try_into().ok())
            .ok_or_else(|| self.error("unexpected end of file"))?;
        self.pos += 4;
        Ok(i32::from_le_bytes(raw))
    }

    fn error(&self, message: &str) -> SteamError {
        SteamError::Parse {
            what: "shortcuts.vdf".to_string(),
            message: format!("{message} at byte {}", self.pos),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/steam")
    }

    fn string(key: &str, value: &str) -> Vec<u8> {
        let mut out = vec![0x01];
        out.extend_from_slice(key.as_bytes());
        out.push(0x00);
        out.extend_from_slice(value.as_bytes());
        out.push(0x00);
        out
    }

    fn int(key: &str, value: i32) -> Vec<u8> {
        let mut out = vec![0x02];
        out.extend_from_slice(key.as_bytes());
        out.push(0x00);
        out.extend_from_slice(&value.to_le_bytes());
        out
    }

    fn map(key: &str, body: &[Vec<u8>]) -> Vec<u8> {
        let mut out = vec![0x00];
        out.extend_from_slice(key.as_bytes());
        out.push(0x00);
        for part in body {
            out.extend_from_slice(part);
        }
        out.push(0x08);
        out
    }

    fn file(entries: &[Vec<u8>]) -> Vec<u8> {
        let mut out = map("shortcuts", entries);
        out.push(0x08);
        out
    }

    fn non_steam(appid: u32, name: &str) -> LibraryEntry {
        LibraryEntry {
            appid,
            name: name.to_string(),
            source: EntrySource::NonSteam,
            installed: true,
        }
    }

    #[test]
    fn hand_built_file_with_negative_appid() {
        let bytes = file(&[map(
            "0",
            &[
                int("appid", -1_294_967_296),
                string("AppName", "Gibson Terminal"),
                string("Exe", "\"/opt/gibson/run.sh\""),
                int("IsHidden", 0),
                map("tags", &[string("0", "Hacking")]),
            ],
        )]);
        assert_eq!(
            parse_shortcuts(&bytes).unwrap(),
            vec![non_steam(3_000_000_000, "Gibson Terminal")]
        );
    }

    #[test]
    fn missing_appid_uses_crc32_fallback() {
        let bytes = file(&[map(
            "0",
            &[string("AppName", "Foo"), string("Exe", "\"/usr/bin/foo\"")],
        )]);
        assert_eq!(
            parse_shortcuts(&bytes).unwrap(),
            vec![non_steam(3_810_698_013, "Foo")]
        );
    }

    #[test]
    fn fallback_appid_value() {
        assert_eq!(fallback_appid("\"/usr/bin/foo\"", "Foo"), 3_810_698_013);
    }

    #[test]
    fn lower_case_keys_are_read() {
        let bytes = file(&[map(
            "0",
            &[
                int("appid", 7),
                string("appname", "Old Format"),
                string("exe", "x"),
            ],
        )]);
        assert_eq!(
            parse_shortcuts(&bytes).unwrap(),
            vec![non_steam(7, "Old Format")]
        );
    }

    #[test]
    fn empty_shortcuts_list() {
        assert!(parse_shortcuts(&file(&[])).unwrap().is_empty());
    }

    #[test]
    fn unknown_type_byte_is_an_error() {
        let mut bytes = map("shortcuts", &[]);
        bytes.insert(bytes.len() - 1, 0x09);
        let err = parse_shortcuts(&bytes).unwrap_err();
        assert!(matches!(err, SteamError::Parse { .. }));
        assert!(err.to_string().contains("unknown type byte 0x09"), "{err}");
    }

    #[test]
    fn truncated_file_is_an_error() {
        let bytes = file(&[map("0", &[int("appid", 7)])]);
        assert!(parse_shortcuts(&bytes[..bytes.len() - 6]).is_err());
    }

    #[test]
    fn fixture_file_sorted_by_name() {
        assert_eq!(
            load_shortcuts(&fixture_root(), 39_734_273).unwrap(),
            vec![
                non_steam(3_601_430_685, "Da Vinci Virus"),
                non_steam(3_000_000_000, "Gibson Terminal"),
            ]
        );
    }

    #[test]
    fn missing_file_means_no_shortcuts() {
        let dir = tempfile::TempDir::new().unwrap();
        assert!(load_shortcuts(dir.path(), 1).unwrap().is_empty());
    }

    #[test]
    fn shortcuts_path_layout() {
        let root = PathBuf::from("/steam");
        assert_eq!(
            shortcuts_path(&root, 42),
            root.join("userdata")
                .join("42")
                .join("config")
                .join("shortcuts.vdf")
        );
    }
}
