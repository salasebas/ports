//! Icons of projects and pages, served to the windows by content hash, so
//! snapshots carry a short key instead of the image.

use std::collections::HashMap;
use std::fs;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::SystemTime;

use tauri::http::{Response, StatusCode, header};

/// The URI scheme the windows load icons from. Must match `src/lib/ports/format.ts`.
pub const SCHEME: &str = "ports-icon";
/// Favicons and project icons are small; anything bigger isn't one.
pub const MAX_BYTES: u64 = 256 * 1024;

#[derive(Debug)]
pub struct Icon {
    pub key: String,
    mime: String,
    bytes: Vec<u8>,
}

/// Every icon something still holds: a page check or a project icon file.
/// An icon nobody holds anymore is dropped.
#[derive(Default)]
pub struct Icons(Mutex<HashMap<String, Weak<Icon>>>);

impl Icons {
    pub fn insert(&self, mime: &str, bytes: Vec<u8>) -> Arc<Icon> {
        let mut hasher = DefaultHasher::new();
        mime.hash(&mut hasher);
        bytes.hash(&mut hasher);
        let key = format!("{:016x}", hasher.finish());
        let mut icons = super::lock(&self.0);
        if let Some(icon) = icons.get(&key).and_then(Weak::upgrade) {
            return icon;
        }
        icons.retain(|_, icon| icon.strong_count() > 0);
        let icon = Arc::new(Icon {
            key: key.clone(),
            mime: mime.to_string(),
            bytes,
        });
        icons.insert(key, Arc::downgrade(&icon));
        icon
    }

    fn get(&self, key: &str) -> Option<Arc<Icon>> {
        super::lock(&self.0).get(key).and_then(Weak::upgrade)
    }

    /// Answers `ports-icon://localhost/<key>`. The key changes with the
    /// image, so the window may cache it for good.
    pub fn respond(&self, path: &str) -> Response<Vec<u8>> {
        // An SVG runs no script inside `<img>`; the policy also covers opening it directly.
        let response = Response::builder()
            .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
            .header(
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; style-src 'unsafe-inline'; sandbox",
            );
        match self.get(path.trim_start_matches('/')) {
            Some(icon) => response
                .header(header::CONTENT_TYPE, &icon.mime)
                .header(header::CACHE_CONTROL, "max-age=31536000, immutable")
                .body(icon.bytes.clone()),
            None => response.status(StatusCode::NOT_FOUND).body(Vec::new()),
        }
        .unwrap_or_default()
    }
}

/// The image types a window shows, by file extension.
fn mime(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => return None,
    })
}

/// A project's icon file as last read, kept until the file changes.
struct Read {
    modified: Option<SystemTime>,
    len: u64,
    icon: Option<Arc<Icon>>,
}

/// Project icon files, read again only when they change on disk.
#[derive(Default)]
pub struct Files {
    read: HashMap<PathBuf, Read>,
}

impl Files {
    pub fn icon(&mut self, path: &Path, icons: &Icons) -> Option<Arc<Icon>> {
        let metadata = fs::metadata(path).ok()?;
        let modified = metadata.modified().ok();
        if let Some(read) = self.read.get(path)
            && read.modified == modified
            && read.len == metadata.len()
        {
            return read.icon.clone();
        }
        let icon = mime(path)
            .filter(|_| metadata.len() <= MAX_BYTES)
            .and_then(|mime| Some(icons.insert(mime, fs::read(path).ok()?)));
        self.read.insert(
            path.to_path_buf(),
            Read {
                modified,
                len: metadata.len(),
                icon: icon.clone(),
            },
        );
        icon
    }

    /// Forgets the files no server uses anymore, which lets their icons go.
    pub fn retain(&mut self, used: &[PathBuf]) {
        self.read.retain(|path, _| used.contains(path));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_an_icon_while_something_holds_it() {
        let icons = Icons::default();
        let icon = icons.insert("image/svg+xml", b"<svg/>".to_vec());
        let response = icons.respond(&format!("/{}", icon.key));
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/svg+xml");
        assert_eq!(response.body(), b"<svg/>");
        // The same image gets the same key, so the window's cache keeps working.
        assert_eq!(
            icons.insert("image/svg+xml", b"<svg/>".to_vec()).key,
            icon.key
        );
        let key = icon.key.clone();
        drop(icon);
        assert_eq!(icons.respond(&key).status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn rereads_a_project_icon_only_after_it_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("favicon.svg");
        fs::write(&path, "<svg>1</svg>").unwrap();
        let icons = Icons::default();
        let mut files = Files::default();
        let first = files.icon(&path, &icons).unwrap();
        assert_eq!(files.icon(&path, &icons).unwrap().key, first.key);
        fs::write(&path, "<svg>two</svg>").unwrap();
        assert_ne!(files.icon(&path, &icons).unwrap().key, first.key);
        // Not an image a window shows.
        let text = dir.path().join("icon.txt");
        fs::write(&text, "icon").unwrap();
        assert!(files.icon(&text, &icons).is_none());
    }
}
