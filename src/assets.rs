/// One kind of custom Steam library art.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AssetKind {
    Portrait,
    Wide,
    Hero,
    Logo,
}

impl AssetKind {
    pub const ALL: [AssetKind; 4] = [
        AssetKind::Portrait,
        AssetKind::Wide,
        AssetKind::Hero,
        AssetKind::Logo,
    ];

    /// The exact pixel size Steam expects. A logo has no fixed size.
    pub fn target_size(self) -> Option<(u32, u32)> {
        match self {
            AssetKind::Portrait => Some((600, 900)),
            AssetKind::Wide => Some((920, 430)),
            AssetKind::Hero => Some((1920, 620)),
            AssetKind::Logo => None,
        }
    }

    /// The grid file name without its extension, for example `440p`.
    pub fn grid_file_stem(self, appid: u32) -> String {
        match self {
            AssetKind::Portrait => format!("{appid}p"),
            AssetKind::Wide => format!("{appid}"),
            AssetKind::Hero => format!("{appid}_hero"),
            AssetKind::Logo => format!("{appid}_logo"),
        }
    }

    /// The name the UI shows for this kind.
    pub fn label(self) -> &'static str {
        match self {
            AssetKind::Portrait => "Portrait capsule",
            AssetKind::Wide => "Wide capsule",
            AssetKind::Hero => "Hero",
            AssetKind::Logo => "Logo",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_sizes() {
        assert_eq!(AssetKind::Portrait.target_size(), Some((600, 900)));
        assert_eq!(AssetKind::Wide.target_size(), Some((920, 430)));
        assert_eq!(AssetKind::Hero.target_size(), Some((1920, 620)));
        assert_eq!(AssetKind::Logo.target_size(), None);
    }

    #[test]
    fn portrait_file_stem() {
        assert_eq!(AssetKind::Portrait.grid_file_stem(440), "440p");
    }

    #[test]
    fn wide_file_stem() {
        assert_eq!(AssetKind::Wide.grid_file_stem(440), "440");
    }

    #[test]
    fn hero_file_stem() {
        assert_eq!(AssetKind::Hero.grid_file_stem(440), "440_hero");
    }

    #[test]
    fn logo_file_stem() {
        assert_eq!(AssetKind::Logo.grid_file_stem(440), "440_logo");
    }

    #[test]
    fn file_stem_uses_full_u32_appid() {
        assert_eq!(
            AssetKind::Portrait.grid_file_stem(3_221_225_472),
            "3221225472p"
        );
    }

    #[test]
    fn labels() {
        assert_eq!(AssetKind::Portrait.label(), "Portrait capsule");
        assert_eq!(AssetKind::Wide.label(), "Wide capsule");
        assert_eq!(AssetKind::Hero.label(), "Hero");
        assert_eq!(AssetKind::Logo.label(), "Logo");
    }
}
