//! The preview's bodies laid out at 412 dp (plan 20261028_05, section 11,
//! and the 23:00 EDT amendment's tests): Markdown, plain text, a diff and a
//! `git show`, their scrollbar labels and the Always rule, an image, a
//! refusal and Try again, and a Markdown link to another file.

use std::time::Duration;

use super::*;

/// An S23's 412 dp at 2.625.
const PHONE: Layout = Layout {
    w: 412.0 * 2.625,
    h: 915.0 * 2.625,
    scale: 2.625,
    top: 0.0,
    bottom: 0.0,
};

/// Opens `target`, then the file `bytes` of `mime` arrives.
fn shown(target: Target, mime: &str, name: &str, bytes: &[u8]) -> (Preview, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fl-0.bin");
    std::fs::write(&path, bytes).unwrap();
    let mut p = Preview::new(None);
    let input = p.open(target.clone(), "w1:p1", Some("flankr"));
    let (ns, request) = match &input {
        Input::RequestFile { path, .. } => ("files", path.clone()),
        Input::RequestGit { request, .. } => ("git", request.clone()),
        other => panic!("{other:?}"),
    };
    p.on_event(&Event::TransferStarted {
        id: 3,
        ns: ns.into(),
        session: "w1:p1".into(),
        request,
    });
    assert_eq!(p.shows(), "loading");
    p.on_event(&Event::FileReady {
        id: 3,
        path,
        mime: mime.into(),
        name: name.into(),
        size: bytes.len() as u64,
    });
    (p, dir)
}

/// Ticks until the decoder is done.
fn decoded(p: &mut Preview) {
    for _ in 0..500 {
        p.tick(Duration::from_millis(10));
        if p.shows() != "decoding" {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn markdown_shows_with_its_headings_as_labels() {
    let md = "# The plan\n\nSome text.\n\n## 1. Goal\n\nMore.\n\n### Not a label\n";
    let (mut p, _d) = shown(
        Target::File("docs/design/x.md".into()),
        "text/markdown",
        "docs/design/x.md",
        md.as_bytes(),
    );
    assert_eq!(p.shows(), "markdown");
    let (marks, always) = p.bookmarks(&PHONE);
    let texts: Vec<&str> = marks.iter().map(|m| m.1.as_str()).collect();
    assert_eq!(texts, ["The plan", "1. Goal"]);
    assert!(marks[0].0 < marks[1].0, "{marks:?}");
    assert!(!always);
    let mut list = DrawList::new();
    p.draw(&PHONE, &DrawContext::default(), &mut list);
    assert!(!list.instances.is_empty());
}

#[test]
fn plain_text_has_no_labels_and_wraps_with_a_mark() {
    let line = "x".repeat(400);
    let (mut p, _d) = shown(
        Target::File("docs/notes.txt".into()),
        "text/plain",
        "docs/notes.txt",
        format!("{line}\nshort\n").as_bytes(),
    );
    assert_eq!(p.shows(), "text");
    let rows = p.rows(&PHONE);
    assert!(rows.len() > 2);
    assert!(rows[0].wraps && !rows.last().unwrap().wraps);
    assert_eq!(rows.last().unwrap().text, "short");
    let (marks, always) = p.bookmarks(&PHONE);
    assert!(marks.is_empty());
    assert!(!always);
}

#[test]
fn a_long_text_shows_its_thumb_always() {
    let text = "line\n".repeat(2000);
    let (mut p, _d) = shown(
        Target::File("docs/log.txt".into()),
        "text/plain",
        "docs/log.txt",
        text.as_bytes(),
    );
    let (_, always) = p.bookmarks(&PHONE);
    assert!(always, "2000 rows are over three screens");
}

#[test]
fn a_git_show_is_a_diff_labelled_by_commit_and_file() {
    let show = "commit 1a2b3c4d5e6f7a8b\nAuthor: x\n\n    msg\n\ndiff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/docs/b.md b/docs/b.md\n+b\n";
    let (mut p, _d) = shown(
        Target::show("1a2b3c4d"),
        "text/x-diff",
        "show 1a2b3c4d",
        show.as_bytes(),
    );
    assert_eq!(p.shows(), "diff");
    let rows = p.rows(&PHONE);
    let colour = |t: &str| rows.iter().find(|r| r.text == t).unwrap().color;
    assert_eq!(colour("+new"), theme().green);
    assert_eq!(colour("-old"), theme().red);
    assert_eq!(colour("@@ -1 +1 @@"), theme().accent);
    assert_eq!(colour("+++ b/src/a.rs"), theme().dim);
    let (marks, _) = p.bookmarks(&PHONE);
    let texts: Vec<&str> = marks.iter().map(|m| m.1.as_str()).collect();
    assert_eq!(texts, ["commit 1a2b3c4d", "a.rs", "b.md"]);
    assert_eq!(marks[0].0, 0.0);
    assert!(marks[1].0 < marks[2].0);
}

#[test]
#[cfg(not(target_os = "android"))]
fn a_png_shows_as_an_image_and_a_jpeg_says_why_not() {
    let mut png_bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png_bytes, 40, 20);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()
            .unwrap()
            .write_image_data(&[128; 40 * 20 * 4])
            .unwrap();
    }
    let (mut p, _d) = shown(
        Target::File("docs/a.png".into()),
        "image/png",
        "docs/a.png",
        &png_bytes,
    );
    decoded(&mut p);
    assert_eq!(p.shows(), "image 40x20");
    let (_, _, rect, clip) = p.image(&PHONE).unwrap();
    // Fit to the width.
    assert!((rect.w - PHONE.w).abs() < 0.5, "{rect:?}");
    assert_eq!(clip.w, PHONE.w);
    let (mut p, _d) = shown(
        Target::File("docs/a.jpg".into()),
        "image/jpeg",
        "docs/a.jpg",
        &[0xff, 0xd8, 0xff, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    );
    decoded(&mut p);
    assert_eq!(
        p.shows(),
        format!("note {}", copy::PREVIEW_IMAGE_PNG_ONLY.text)
    );
}

#[test]
fn a_refusal_shows_its_line_and_try_again_asks_again() {
    let mut p = Preview::new(None);
    let first = p.open(Target::File("docs/x.md".into()), "w1:p1", None);
    p.on_event(&Event::TransferStarted {
        id: 9,
        ns: "files".into(),
        session: "w1:p1".into(),
        request: "docs/x.md".into(),
    });
    // Another transfer's failure is not its.
    p.on_event(&Event::TransferFailed {
        id: 8,
        code: 8,
        line: Some("File not found.".into()),
    });
    assert_eq!(p.shows(), "loading");
    p.on_event(&Event::TransferFailed {
        id: 9,
        code: 8,
        line: Some("File not found.".into()),
    });
    assert_eq!(p.shows(), "failed File not found.");
    let r = Preview::retry_rect(&Preview::frame(&PHONE));
    let [x, y] = r.center();
    let now = Duration::ZERO;
    assert_eq!(p.touch(&PHONE, (1, TouchPhase::Started, x, y), now), None);
    assert_eq!(
        p.touch(&PHONE, (1, TouchPhase::Ended, x, y), now),
        Some(PreviewHit::Send(first))
    );
    assert_eq!(p.shows(), "loading");
    // The ✕ closes it.
    let c = Preview::frame(&PHONE).close.center();
    p.touch(&PHONE, (2, TouchPhase::Started, c[0], c[1]), now);
    assert_eq!(
        p.touch(&PHONE, (2, TouchPhase::Ended, c[0], c[1]), now),
        Some(PreviewHit::Closed)
    );
    assert!(!p.is_open());
}

#[test]
fn relative_markdown_links_resolve_against_the_files_folder() {
    assert_eq!(
        resolve("docs/design", "x.md").as_deref(),
        Some("docs/design/x.md")
    );
    assert_eq!(
        resolve("docs/design", "../README.md").as_deref(),
        Some("docs/README.md")
    );
    assert_eq!(
        resolve("docs/design", "./y.md#part").as_deref(),
        Some("docs/design/y.md")
    );
    assert_eq!(resolve("", "../x.md"), None);
    assert_eq!(resolve("docs", "https://x/y.md"), None);
    assert_eq!(resolve("docs", "a.png"), None);
}

#[test]
fn a_cut_git_output_says_so_and_sizes_read_well() {
    let text = format!("diff --git a/a b/a\n+x{GIT_CUT_MARKER}");
    let (p, _d) = shown(
        Target::Git("diff".into()),
        "text/x-diff",
        "diff",
        text.as_bytes(),
    );
    let sub = p.open.as_ref().unwrap().sub.clone();
    assert!(sub.starts_with("flankr \u{00B7} "), "{sub}");
    assert!(sub.ends_with(copy::PREVIEW_GIT_CUT.text), "{sub}");
    assert_eq!(format_size(900), "900 B");
    assert_eq!(format_size(12 * 1024), "12 KB");
    assert_eq!(format_size(3 << 20), "3.0 MB");
}
