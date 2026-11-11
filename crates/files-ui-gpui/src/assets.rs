//! Icons: ours (`assets/icons`: a few Lucide file and folder icons, ISC, and
//! the app logo) first, then gpui-kit's default set. Embedding Lucide's full
//! catalog instead would add 2.8 MB for a dozen icons.

use gpui_kit::{AssetSource, Result, SharedString};
use std::borrow::Cow;

macro_rules! own {
    ($($name:literal),* $(,)?) => {
        &[$((
            concat!("icons/", $name, ".svg"),
            include_bytes!(concat!("../assets/icons/", $name, ".svg")) as &[u8],
        )),*]
    };
}

const OWN: &[(&str, &[u8])] = own!(
    "app",
    "monitor",
    "trash",
    "file-archive",
    "file-code",
    "file-cog",
    "file-image",
    "file-key",
    "file-music",
    "file-play",
    "file-spreadsheet",
    "file-terminal",
    "file-text",
    "file-type",
    "folder-dot",
    "folder-down",
    "folder-git-2",
);

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match OWN.iter().find(|(p, _)| *p == path) {
            Some((_, bytes)) => Ok(Some(Cow::Borrowed(bytes))),
            None => gpui_kit::assets::Assets.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        gpui_kit::assets::Assets.list(path)
    }
}
