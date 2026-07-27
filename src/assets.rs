use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

pub(crate) struct Assets;

const ASSET_PATHS: [&str; 15] = [
    "multiple.svg",
    "launch.svg",
    "remove.svg",
    "spinner.svg",
    "stop.svg",
    "signin.svg",
    "info.svg",
    "star.svg",
    "star-filled.svg",
    "search.svg",
    "people.svg",
    "close.svg",
    "settings.svg",
    "appearance.svg",
    "data.svg",
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        let bytes: Option<&'static [u8]> = match path {
            "multiple.svg" => Some(include_bytes!("../assets/multiple.svg")),
            "launch.svg" => Some(include_bytes!("../assets/launch.svg")),
            "remove.svg" => Some(include_bytes!("../assets/remove.svg")),
            "spinner.svg" => Some(include_bytes!("../assets/spinner.svg")),
            "stop.svg" => Some(include_bytes!("../assets/stop.svg")),
            "signin.svg" => Some(include_bytes!("../assets/signin.svg")),
            "info.svg" => Some(include_bytes!("../assets/info.svg")),
            "star.svg" => Some(include_bytes!("../assets/star.svg")),
            "star-filled.svg" => Some(include_bytes!("../assets/star-filled.svg")),
            "search.svg" => Some(include_bytes!("../assets/search.svg")),
            "people.svg" => Some(include_bytes!("../assets/people.svg")),
            "close.svg" => Some(include_bytes!("../assets/close.svg")),
            "settings.svg" => Some(include_bytes!("../assets/settings.svg")),
            "appearance.svg" => Some(include_bytes!("../assets/appearance.svg")),
            "data.svg" => Some(include_bytes!("../assets/data.svg")),
            _ => None,
        };

        Ok(bytes.map(Cow::Borrowed))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        if path.is_empty() {
            Ok(ASSET_PATHS.into_iter().map(SharedString::from).collect())
        } else {
            Ok(Vec::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_declared_runtime_asset_loads_nonempty_svg() {
        let assets = Assets;

        for path in ASSET_PATHS {
            let bytes = assets
                .load(path)
                .expect("asset loading should succeed")
                .expect("declared asset should exist");

            assert!(!bytes.is_empty(), "{path} should not be empty");
            assert!(
                std::str::from_utf8(&bytes)
                    .expect("SVG should be UTF-8")
                    .contains("<svg"),
                "{path} should contain an SVG root"
            );
        }
    }

    #[test]
    fn asset_listing_exposes_only_runtime_assets() {
        let listed = Assets
            .list("")
            .expect("asset listing should succeed")
            .into_iter()
            .map(|path| path.to_string())
            .collect::<Vec<_>>();

        assert_eq!(listed, ASSET_PATHS);
        assert!(
            Assets
                .list("nested")
                .expect("nested listing should succeed")
                .is_empty()
        );
        assert!(
            Assets
                .load("unknown.svg")
                .expect("unknown asset lookup should succeed")
                .is_none()
        );
    }
}
