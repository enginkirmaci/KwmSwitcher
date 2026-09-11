//! Asset source: our own logo/icons plus gpui-kit's bundled icon set for the
//! component library's internal icons.

use std::borrow::Cow;

use gpui_kit::{AssetSource, SharedString};

pub struct AppAssets;

fn embedded(path: &str) -> Option<&'static [u8]> {
    match path {
        "logo.png" => Some(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/logo.png"))),
        "logo.svg" => Some(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/logo.svg"))),
        "icons/laptop.svg" => {
            Some(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/laptop.svg")))
        }
        "icons/monitor.svg" => {
            Some(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/monitor.svg")))
        }
        "icons/monitor-smartphone.svg" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/icons/monitor-smartphone.svg"
        ))),
        "icons/server.svg" => {
            Some(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/server.svg")))
        }
        "icons/picture-in-picture-2.svg" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/icons/picture-in-picture-2.svg"
        ))),
        "icons/usb.svg" => {
            Some(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/usb.svg")))
        }
        "icons/power.svg" => {
            Some(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/power.svg")))
        }
        "icons/refresh-cw.svg" => {
            Some(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/refresh-cw.svg")))
        }
        "icons/arrow-left.svg" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/icons/arrow-left.svg"
        ))),
        "icons/arrow-right.svg" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/icons/arrow-right.svg"
        ))),
        "icons/arrow-left-right.svg" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/icons/arrow-left-right.svg"
        ))),
        "icons/columns-2.svg" => {
            Some(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/columns-2.svg")))
        }
        "icons/toggle-left.svg" => Some(include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/icons/toggle-left.svg"
        ))),
        "icons/settings.svg" => {
            Some(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/settings.svg")))
        }
        _ => None,
    }
}

impl AssetSource for AppAssets {
    fn load(&self, path: &str) -> gpui_kit::Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = embedded(path) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui_kit::Result<Vec<SharedString>> {
        gpui_kit::assets::Assets.list(path)
    }
}
