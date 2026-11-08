//! Crop and resize art to the sizes Steam expects.

use image::DynamicImage;
use image::imageops::FilterType;

use crate::assets::AssetKind;

/// Center-crops `image` to the aspect ratio of `size`, then resizes it to
/// exactly `size` with Lanczos3.
pub fn fit_exact(image: &DynamicImage, size: (u32, u32)) -> DynamicImage {
    let (width, height) = (u64::from(image.width()), u64::from(image.height()));
    let (target_width, target_height) = (u64::from(size.0), u64::from(size.1));
    // Compare width / height with target_width / target_height without
    // floats: the larger cross product tells which side is too long.
    let (crop_width, crop_height) = if width * target_height > height * target_width {
        ((height * target_width / target_height).max(1), height)
    } else {
        (width, (width * target_height / target_width).max(1))
    };
    let x = (width - crop_width) / 2;
    let y = (height - crop_height) / 2;
    // Every value fits in u32: each is at most the source width or height.
    let cropped = image.crop_imm(x as u32, y as u32, crop_width as u32, crop_height as u32);
    cropped.resize_exact(size.0, size.1, FilterType::Lanczos3)
}

/// The size in `supported` with the aspect ratio nearest to `target`.
/// On a tie, the first size in `supported` wins.
///
/// # Panics
///
/// Panics if `supported` is empty.
pub fn nearest_openai_size(target: (u32, u32), supported: &[(u32, u32)]) -> (u32, u32) {
    let ratio = |(width, height): (u32, u32)| f64::from(width) / f64::from(height);
    let target_ratio = ratio(target);
    // The log of the ratio quotient treats 2:1 and 1:2 as equally far from 1:1.
    let distance = |size: (u32, u32)| (ratio(size) / target_ratio).ln().abs();
    supported
        .iter()
        .copied()
        .reduce(|best, size| {
            if distance(size) < distance(best) {
                size
            } else {
                best
            }
        })
        .expect("no supported sizes")
}

/// Removes the fully transparent rows and columns at the edges of `image`.
/// A fully transparent image stays as it is.
pub fn trim_transparent(image: &DynamicImage) -> DynamicImage {
    let rgba = image.to_rgba8();
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for (x, y, pixel) in rgba.enumerate_pixels() {
        if pixel[3] == 0 {
            continue;
        }
        bounds = Some(match bounds {
            None => (x, y, x, y),
            Some((left, top, right, bottom)) => {
                (left.min(x), top.min(y), right.max(x), bottom.max(y))
            }
        });
    }
    match bounds {
        Some((left, top, right, bottom)) => {
            image.crop_imm(left, top, right - left + 1, bottom - top + 1)
        }
        None => image.clone(),
    }
}

/// Makes `image` ready to write as `kind`: the exact Steam size for the
/// capsules and the hero, trimmed transparent borders for the logo.
pub fn prepare(image: &DynamicImage, kind: AssetKind) -> DynamicImage {
    match kind.target_size() {
        Some(size) => fit_exact(image, size),
        None => trim_transparent(image),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    const RED: Rgba<u8> = Rgba([255, 0, 0, 255]);
    const BLUE: Rgba<u8> = Rgba([0, 0, 255, 255]);
    const CLEAR: Rgba<u8> = Rgba([0, 0, 0, 0]);

    /// An image that is blue inside the rectangle and `outside` elsewhere.
    fn with_rect(size: (u32, u32), rect: (u32, u32, u32, u32), outside: Rgba<u8>) -> DynamicImage {
        let (x0, y0, w, h) = rect;
        let img = RgbaImage::from_fn(size.0, size.1, |x, y| {
            if (x0..x0 + w).contains(&x) && (y0..y0 + h).contains(&y) {
                BLUE
            } else {
                outside
            }
        });
        DynamicImage::ImageRgba8(img)
    }

    fn dims(image: &DynamicImage) -> (u32, u32) {
        (image.width(), image.height())
    }

    #[test]
    fn fit_exact_gives_every_steam_size() {
        let source = with_rect((1024, 1024), (0, 0, 0, 0), RED);
        for kind in [AssetKind::Portrait, AssetKind::Wide, AssetKind::Hero] {
            let size = kind.target_size().unwrap();
            assert_eq!(dims(&fit_exact(&source, size)), size, "{kind:?}");
        }
    }

    #[test]
    fn fit_exact_from_a_wide_source() {
        let source = with_rect((1536, 1024), (0, 0, 0, 0), RED);
        assert_eq!(dims(&fit_exact(&source, (600, 900))), (600, 900));
    }

    #[test]
    fn fit_exact_crops_the_sides_of_a_wide_source_evenly() {
        // 300x100 to a square: the center 100x100 is blue, the sides are red.
        let source = with_rect((300, 100), (100, 0, 100, 100), RED);
        let fitted = fit_exact(&source, (50, 50)).to_rgba8();
        assert_eq!(fitted.dimensions(), (50, 50));
        assert!(fitted.pixels().all(|pixel| pixel == &BLUE));
    }

    #[test]
    fn fit_exact_crops_top_and_bottom_of_a_tall_source_evenly() {
        let source = with_rect((100, 300), (0, 100, 100, 100), RED);
        let fitted = fit_exact(&source, (40, 40)).to_rgba8();
        assert_eq!(fitted.dimensions(), (40, 40));
        assert!(fitted.pixels().all(|pixel| pixel == &BLUE));
    }

    #[test]
    fn fit_exact_keeps_the_center_pixel() {
        // A 2:1 target from a square: rows 25..75 stay. The blue band sits
        // at rows 45..55, so it must end up at the middle rows.
        let source = with_rect((100, 100), (0, 45, 100, 10), RED);
        let fitted = fit_exact(&source, (100, 50)).to_rgba8();
        assert_eq!(fitted.get_pixel(50, 25), &BLUE);
        assert_eq!(fitted.get_pixel(50, 2), &RED);
        assert_eq!(fitted.get_pixel(50, 47), &RED);
    }

    #[test]
    fn nearest_size_for_each_steam_kind() {
        let supported = [(1024, 1024), (1024, 1536), (1536, 1024)];
        assert_eq!(nearest_openai_size((600, 900), &supported), (1024, 1536));
        assert_eq!(nearest_openai_size((920, 430), &supported), (1536, 1024));
        assert_eq!(nearest_openai_size((1920, 620), &supported), (1536, 1024));
        assert_eq!(nearest_openai_size((512, 500), &supported), (1024, 1024));
    }

    #[test]
    fn nearest_size_compares_ratios_not_pixels() {
        // 2:1 is nearer to 3:1 than 1:1 is, by ratio (ln 1.5 < ln 3).
        let supported = [(300, 300), (2000, 1000)];
        assert_eq!(nearest_openai_size((300, 100), &supported), (2000, 1000));
    }

    #[test]
    fn nearest_size_tie_takes_the_first() {
        let supported = [(200, 100), (100, 200)];
        assert_eq!(nearest_openai_size((100, 100), &supported), (200, 100));
    }

    #[test]
    #[should_panic(expected = "no supported sizes")]
    fn nearest_size_panics_without_sizes() {
        nearest_openai_size((600, 900), &[]);
    }

    #[test]
    fn trim_removes_transparent_borders() {
        let logo = with_rect((100, 60), (10, 20, 30, 15), CLEAR);
        let trimmed = trim_transparent(&logo).to_rgba8();
        assert_eq!(trimmed.dimensions(), (30, 15));
        assert!(trimmed.pixels().all(|pixel| pixel == &BLUE));
    }

    #[test]
    fn trim_keeps_partly_transparent_pixels() {
        let mut logo = RgbaImage::from_pixel(10, 10, CLEAR);
        logo.put_pixel(2, 3, Rgba([0, 0, 0, 1]));
        logo.put_pixel(7, 8, Rgba([0, 0, 0, 1]));
        let trimmed = trim_transparent(&DynamicImage::ImageRgba8(logo));
        assert_eq!(dims(&trimmed), (6, 6));
    }

    #[test]
    fn trim_leaves_an_opaque_image_as_it_is() {
        let logo = with_rect((8, 4), (0, 0, 8, 4), CLEAR);
        assert_eq!(trim_transparent(&logo), logo);
    }

    #[test]
    fn trim_leaves_a_fully_transparent_image_as_it_is() {
        let logo = with_rect((8, 4), (0, 0, 0, 0), CLEAR);
        assert_eq!(trim_transparent(&logo), logo);
    }

    #[test]
    fn prepare_fits_sized_kinds() {
        let source = with_rect((1024, 1024), (0, 0, 0, 0), RED);
        assert_eq!(dims(&prepare(&source, AssetKind::Hero)), (1920, 620));
    }

    #[test]
    fn prepare_trims_a_logo_and_keeps_its_size() {
        let logo = with_rect((1024, 1024), (100, 400, 800, 200), CLEAR);
        let prepared = prepare(&logo, AssetKind::Logo);
        assert_eq!(dims(&prepared), (800, 200));
        assert!(prepared.color().has_alpha());
    }
}
