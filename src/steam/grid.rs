//! Custom art files in `<root>/userdata/<accountid>/config/grid/`.

use std::path::{Path, PathBuf};

use image::DynamicImage;

use crate::assets::AssetKind;

/// The extensions Steam reads for a grid file, in the order the app checks
/// them. The app writes PNG only.
const EXTENSIONS: [&str; 2] = ["png", "jpg"];

#[derive(Debug, thiserror::Error)]
pub enum GridError {
    #[error("cannot write {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot encode {} as PNG: {source}", path.display())]
    Encode {
        path: PathBuf,
        #[source]
        source: image::ImageError,
    },
}

/// The grid folder of one Steam account.
pub fn grid_dir(steam_root: &Path, account_id: u32) -> PathBuf {
    steam_root
        .join("userdata")
        .join(account_id.to_string())
        .join("config")
        .join("grid")
}

fn slot_path(grid_dir: &Path, appid: u32, kind: AssetKind, extension: &str) -> PathBuf {
    grid_dir.join(format!("{}.{extension}", kind.grid_file_stem(appid)))
}

/// The custom file for one slot, if the grid folder has one.
pub fn current_custom(grid_dir: &Path, appid: u32, kind: AssetKind) -> Option<PathBuf> {
    EXTENSIONS
        .iter()
        .map(|extension| slot_path(grid_dir, appid, kind, extension))
        .find(|path| path.is_file())
}

/// Writes `image` as the PNG file for one slot and returns its path.
/// Creates the grid folder if it is missing. Replaces an existing file and
/// deletes the JPG file of the same slot, so Steam shows the written file.
pub fn write_asset(
    grid_dir: &Path,
    appid: u32,
    kind: AssetKind,
    image: &DynamicImage,
) -> Result<PathBuf, GridError> {
    let io_error = |path: &Path| {
        let path = path.to_path_buf();
        move |source| GridError::Io { path, source }
    };
    std::fs::create_dir_all(grid_dir).map_err(io_error(grid_dir))?;

    let png = slot_path(grid_dir, appid, kind, "png");
    image
        .save_with_format(&png, image::ImageFormat::Png)
        .map_err(|source| match source {
            image::ImageError::IoError(source) => GridError::Io {
                path: png.clone(),
                source,
            },
            source => GridError::Encode {
                path: png.clone(),
                source,
            },
        })?;

    for extension in EXTENSIONS.iter().filter(|extension| **extension != "png") {
        let twin = slot_path(grid_dir, appid, kind, extension);
        match std::fs::remove_file(&twin) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(io_error(&twin)(err)),
        }
    }
    Ok(png)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};
    use tempfile::TempDir;

    fn solid(width: u32, height: u32, color: [u8; 4]) -> DynamicImage {
        DynamicImage::ImageRgba8(RgbaImage::from_pixel(width, height, Rgba(color)))
    }

    #[test]
    fn grid_dir_is_under_the_account_config() {
        let root = Path::new("steam");
        assert_eq!(
            grid_dir(root, 39_734_273),
            root.join("userdata")
                .join("39734273")
                .join("config")
                .join("grid")
        );
    }

    #[test]
    fn no_custom_file_in_a_missing_folder() {
        let temp = TempDir::new().unwrap();
        let grid = temp.path().join("grid");
        assert_eq!(current_custom(&grid, 440, AssetKind::Hero), None);
    }

    #[test]
    fn finds_png_custom_file() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("440_hero.png");
        std::fs::write(&file, b"png").unwrap();
        assert_eq!(
            current_custom(temp.path(), 440, AssetKind::Hero),
            Some(file)
        );
    }

    #[test]
    fn finds_jpg_custom_file() {
        let temp = TempDir::new().unwrap();
        let file = temp.path().join("440p.jpg");
        std::fs::write(&file, b"jpg").unwrap();
        assert_eq!(
            current_custom(temp.path(), 440, AssetKind::Portrait),
            Some(file)
        );
    }

    #[test]
    fn png_wins_over_jpg() {
        let temp = TempDir::new().unwrap();
        std::fs::write(temp.path().join("440.jpg"), b"jpg").unwrap();
        std::fs::write(temp.path().join("440.png"), b"png").unwrap();
        assert_eq!(
            current_custom(temp.path(), 440, AssetKind::Wide),
            Some(temp.path().join("440.png"))
        );
    }

    #[test]
    fn other_slot_file_does_not_match() {
        let temp = TempDir::new().unwrap();
        std::fs::write(temp.path().join("440p.png"), b"png").unwrap();
        std::fs::write(temp.path().join("440.json"), b"{}").unwrap();
        assert_eq!(current_custom(temp.path(), 440, AssetKind::Wide), None);
        assert_eq!(current_custom(temp.path(), 440, AssetKind::Logo), None);
    }

    #[test]
    fn write_creates_folder_and_png() {
        let temp = TempDir::new().unwrap();
        let grid = temp
            .path()
            .join("userdata")
            .join("1")
            .join("config")
            .join("grid");
        let written = write_asset(
            &grid,
            440,
            AssetKind::Portrait,
            &solid(6, 9, [1, 2, 3, 255]),
        )
        .unwrap();
        assert_eq!(written, grid.join("440p.png"));
        let read = image::open(&written).unwrap();
        assert_eq!((read.width(), read.height()), (6, 9));
        assert_eq!(read.to_rgba8().get_pixel(0, 0), &Rgba([1, 2, 3, 255]));
    }

    #[test]
    fn write_replaces_existing_png() {
        let temp = TempDir::new().unwrap();
        write_asset(
            temp.path(),
            440,
            AssetKind::Hero,
            &solid(2, 2, [255, 0, 0, 255]),
        )
        .unwrap();
        write_asset(
            temp.path(),
            440,
            AssetKind::Hero,
            &solid(3, 1, [0, 255, 0, 255]),
        )
        .unwrap();
        let read = image::open(temp.path().join("440_hero.png")).unwrap();
        assert_eq!((read.width(), read.height()), (3, 1));
    }

    #[test]
    fn write_deletes_jpg_twin() {
        let temp = TempDir::new().unwrap();
        let jpg = temp.path().join("440_logo.jpg");
        std::fs::write(&jpg, b"jpg").unwrap();
        write_asset(
            temp.path(),
            440,
            AssetKind::Logo,
            &solid(1, 1, [0, 0, 0, 0]),
        )
        .unwrap();
        assert!(!jpg.exists());
        assert_eq!(
            current_custom(temp.path(), 440, AssetKind::Logo),
            Some(temp.path().join("440_logo.png"))
        );
    }

    #[test]
    fn write_keeps_other_files() {
        let temp = TempDir::new().unwrap();
        let logo_position = temp.path().join("440.json");
        let other_slot = temp.path().join("440p.jpg");
        std::fs::write(&logo_position, b"{}").unwrap();
        std::fs::write(&other_slot, b"jpg").unwrap();
        write_asset(
            temp.path(),
            440,
            AssetKind::Wide,
            &solid(1, 1, [0, 0, 0, 255]),
        )
        .unwrap();
        assert!(logo_position.exists());
        assert!(other_slot.exists());
    }

    #[test]
    fn write_into_a_file_path_is_an_io_error() {
        let temp = TempDir::new().unwrap();
        let not_a_folder = temp.path().join("grid");
        std::fs::write(&not_a_folder, b"file").unwrap();
        let result = write_asset(&not_a_folder, 440, AssetKind::Wide, &solid(1, 1, [0; 4]));
        assert!(matches!(result, Err(GridError::Io { .. })), "{result:?}");
    }
}
