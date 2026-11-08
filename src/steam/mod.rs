//! Readers for the files and services that hold Steam data.

pub mod library;
pub mod owned;
pub mod paths;
pub mod shortcuts;
pub mod users;

use std::path::{Path, PathBuf};

use keyvalues_parser::{Obj, Value};

#[derive(Debug, thiserror::Error)]
pub enum SteamError {
    #[error("cannot read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot parse {what}: {message}")]
    Parse { what: String, message: String },
}

fn read_text(path: &Path) -> Result<String, SteamError> {
    std::fs::read_to_string(path).map_err(|source| SteamError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Parses text VDF and returns the root object.
fn parse_vdf<'a>(text: &'a str, what: &str) -> Result<Obj<'a>, SteamError> {
    let vdf = keyvalues_parser::parse(text).map_err(|err| SteamError::Parse {
        what: what.to_string(),
        message: err.to_string(),
    })?;
    match vdf.value {
        Value::Obj(obj) => Ok(obj),
        Value::Str(_) => Err(SteamError::Parse {
            what: what.to_string(),
            message: "the root value is not an object".to_string(),
        }),
    }
}

/// The first value of `key`, with the key matched without regard to case.
/// Steam is not consistent about key case (`MostRecent` and `mostrecent`).
fn get_value<'o, 'a>(obj: &'o Obj<'a>, key: &str) -> Option<&'o Value<'a>> {
    obj.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(key))
        .and_then(|(_, values)| values.first())
}

fn get_str<'o>(obj: &'o Obj<'_>, key: &str) -> Option<&'o str> {
    get_value(obj, key).and_then(Value::get_str)
}
