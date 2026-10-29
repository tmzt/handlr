//! The `files` namespace, daemon to phone (docs/design/20261028_05_BINARY_FILES_PLAN.md,
//! decisions 6.1-6.3): a REQUEST's path read under exactly `read_file`'s
//! path rules, whole, as text or an image, at most 8 MiB, from memory.

use std::sync::Arc;

use libflankr_blob::{Code, FILE_MAX};

use super::super::conn::Conn;
use super::{Refusal, local_root, read_patterns, session};
use crate::project::{ProjectFiles, ReadRefusal};

/// What a REQUEST is answered with: (name, MIME, bytes, mtime in ms).
pub type Served = (String, &'static str, Vec<u8>, u64);

/// `path` in `session`'s project, for at most `max_size` bytes.
pub(super) async fn request(
    conn: &Arc<Conn>,
    session_id: &str,
    path: &str,
    max_size: u64,
) -> Result<Served, Refusal> {
    let read = read_patterns(&conn.daemon);
    if read.is_empty() {
        return Err((Code::FILES_OFF, "[llm.uses.planning] read is empty".into()));
    }
    let s = session(conn, session_id).await?;
    let root = local_root(conn, &s)?;
    let max = FILE_MAX.min(max_size);
    let path = path.to_string();
    let served = tokio::task::spawn_blocking(move || serve(&root, read, &path, max))
        .await
        .map_err(|e| (Code::IO, e.to_string()))??;
    // As read_file logs its path: the path as resolved, never contents.
    log::info!(
        "files: '{}' for pane '{}' ({}, {} bytes)",
        served.0,
        s.pane_id,
        served.1,
        served.2.len()
    );
    Ok(served)
}

/// The blocking half: `path` under `root` with the `read` patterns.
pub fn serve(
    root: &str,
    read: Vec<crate::llm::ReadPattern>,
    path: &str,
    max: u64,
) -> Result<Served, Refusal> {
    let files = ProjectFiles::open(root, read).map_err(|e| (Code::NOT_FOUND, e))?;
    let f = files.read_bytes(path, max).map_err(|e| match e {
        ReadRefusal::NotPermitted => (Code::NOT_PERMITTED, "the path may not be read".into()),
        ReadRefusal::NotFound => (Code::NOT_FOUND, "no such file".into()),
        ReadRefusal::TooLarge => (Code::TOO_LARGE, format!("over {max} bytes")),
        ReadRefusal::BadType => (Code::BAD_TYPE, "not text or an image".into()),
        ReadRefusal::Io => (Code::IO, "the file cannot be read".into()),
    })?;
    Ok((f.name, f.mime, f.bytes, f.modified_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patterns(list: &[&str]) -> Vec<crate::llm::ReadPattern> {
        list.iter()
            .map(|p| crate::llm::ReadPattern::parse(p).unwrap())
            .collect()
    }

    fn write(root: &std::path::Path, rel: &str, bytes: &[u8]) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    /// The `files` matrix of section 11.
    #[test]
    fn the_read_file_matrix_with_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("p");
        for rel in [
            "README.md",
            "docs/a.md",
            "src/main.rs",
            ".env",
            ".private/key",
        ] {
            write(&root, rel, b"text");
        }
        write(dir.path(), "x.md", b"outside");
        std::os::unix::fs::symlink(dir.path().join("x.md"), root.join("docs/out.md")).unwrap();
        std::os::unix::fs::symlink(root.join(".env"), root.join("docs/env.md")).unwrap();
        let png = [
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, 1, 2,
        ];
        write(&root, "docs/a.png", &png);
        write(&root, "docs/b.png", &[0, 159, 146, 150, 0xff, 0xfe]);
        write(&root, "docs/big.md", &vec![b'a'; 9 << 20]);
        let r = root.to_str().unwrap();
        let read = || patterns(&["*.md", "docs/**/*.md", "docs/*.png"]);
        let ok = serve(r, read(), "README.md", FILE_MAX).unwrap();
        assert_eq!(
            (ok.0.as_str(), ok.1, ok.2.as_slice()),
            ("README.md", "text/markdown", &b"text"[..])
        );
        assert_eq!(
            serve(r, read(), "docs/a.md", FILE_MAX).unwrap().0,
            "docs/a.md"
        );
        for path in [
            "src/main.rs",
            ".env",
            ".private/key",
            "../x.md",
            "docs/out.md",
            "docs/env.md",
        ] {
            assert_eq!(
                serve(r, read(), path, FILE_MAX).unwrap_err().0,
                Code::NOT_PERMITTED,
                "{path}"
            );
        }
        let png_served = serve(r, read(), "docs/a.png", FILE_MAX).unwrap();
        assert_eq!(png_served.1, "image/png");
        assert_eq!(png_served.2, png);
        assert_eq!(
            serve(r, read(), "docs/b.png", FILE_MAX).unwrap_err().0,
            Code::BAD_TYPE
        );
        assert_eq!(
            serve(r, read(), "docs/big.md", FILE_MAX).unwrap_err().0,
            Code::TOO_LARGE
        );
        assert_eq!(
            serve(r, read(), "docs/none.md", FILE_MAX).unwrap_err().0,
            Code::NOT_FOUND
        );
        // REQUEST's max_size narrows FILE_MAX.
        assert_eq!(
            serve(r, read(), "README.md", 2).unwrap_err().0,
            Code::TOO_LARGE
        );
    }
}
