//! Recent projects on the desktop's recent-files list (LINUX-PLAN L2.2): opening or saving a
//! project adds it to `$XDG_DATA_HOME/recently-used.xbel` (the freedesktop "desktop bookmark"
//! file GTK writes; KDE Plasma's activity manager imports it, so it shows in Dolphin's Recent
//! Files, the launcher's recent documents and GNOME's Recent).
//!
//! No XML crate: our entry is one `<bookmark>` element that we remove (if present) and append
//! again before `</xbel>`; everything else in the file is kept byte for byte. A file we can't
//! understand (no `</xbel>`, too big) is left alone. Written atomically (temporary file + rename).

use std::path::{Path, PathBuf};

/// A recently-used file bigger than this is not touched (GTK caps it far lower).
const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// `$XDG_DATA_HOME/recently-used.xbel`, else `~/.local/share/recently-used.xbel`.
pub fn xbel_path() -> Option<PathBuf> {
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    env("XDG_DATA_HOME").filter(|p| p.is_absolute()).or_else(|| env("HOME").map(|h| h.join(".local/share"))).map(|d| d.join("recently-used.xbel"))
}

const HEADER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xbel version=\"1.0\"\n      xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\"\n      xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\"\n>\n";

/// `existing` (the file's text, empty when it doesn't exist) with `uri` added as the newest
/// FilmCraft project; `now` is an ISO 8601 UTC time. `None` when the file isn't one we can edit.
pub fn add_bookmark(existing: &str, uri: &str, now: &str) -> Option<String> {
    // the URI is percent-encoded (no quotes or angle brackets); refuse anything else
    if !uri.starts_with("file://") || uri.contains(['"', '<', '>', '&', '\'']) || now.contains(['"', '<', '>', '&']) {
        return None;
    }
    let base = if existing.trim().is_empty() { format!("{HEADER}</xbel>\n") } else { existing.to_string() };
    let end = base.rfind("</xbel>")?;
    let (mut body, tail) = (base.get(..end)?.to_string(), base.get(end..)?);
    let mut added = now.to_string();
    let mut count = 0u32;
    // drop our earlier entry for this file (keep when it was first added and how often); GTK
    // escapes fewer characters than we do, so hrefs are compared decoded
    if let Some(start) = find_bookmark(&body, uri) {
        let close = body.get(start..)?.find("</bookmark>")? + start + "</bookmark>".len();
        let old = body.get(start..close)?.to_string();
        if let Some(a) = attr(&old, "added") {
            added = a;
        }
        count = attr(&old, "count").and_then(|c| c.parse().ok()).unwrap_or(0);
        // remove the line's indentation and newline too
        let line_start = body.get(..start)?.rfind('\n').map_or(start, |i| i + 1);
        let line_start = if body.get(line_start..start)?.trim().is_empty() { line_start } else { start };
        let after = if body.get(close..)?.starts_with('\n') { close + 1 } else { close };
        body.replace_range(line_start..after, "");
    }
    let count = count.saturating_add(1);
    let entry = format!(
        "  <bookmark href=\"{uri}\" added=\"{added}\" modified=\"{now}\" visited=\"{now}\">\n    <info>\n      <metadata owner=\"http://freedesktop.org\">\n        <mime:mime-type type=\"application/x-filmcraft-project\"/>\n        <bookmark:applications>\n          <bookmark:application name=\"FilmCraft\" exec=\"&apos;filmcraft %u&apos;\" modified=\"{now}\" count=\"{count}\"/>\n        </bookmark:applications>\n      </metadata>\n    </info>\n  </bookmark>\n"
    );
    if !body.ends_with('\n') {
        body.push('\n');
    }
    Some(format!("{body}{entry}{tail}"))
}

/// Where the `<bookmark href="…"` element for `uri` starts (hrefs compared percent-decoded).
fn find_bookmark(body: &str, uri: &str) -> Option<usize> {
    const OPEN: &str = "<bookmark href=\"";
    let want = percent_decode(uri);
    let mut from = 0;
    while let Some(i) = body.get(from..)?.find(OPEN) {
        let start = from + i;
        let href_start = start + OPEN.len();
        let len = body.get(href_start..)?.find('"')?;
        let href = body.get(href_start..href_start + len)?;
        if percent_decode(&href.replace("&amp;", "&")) == want {
            return Some(start);
        }
        from = href_start + len;
    }
    None
}

/// `%XX` escapes decoded to bytes (malformed escapes are kept as they are).
pub(crate) fn percent_decode(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let hex = |c: u8| char::from(c).to_digit(16).and_then(|d| u8::try_from(d).ok());
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if c == b'%'
            && let (Some(h), Some(l)) = (b.get(i + 1).copied().and_then(hex), b.get(i + 2).copied().and_then(hex))
        {
            out.push(h * 16 + l);
            i += 3;
            continue;
        }
        out.push(c);
        i += 1;
    }
    out
}

/// The value of `name="…"` in `s` (first one).
fn attr(s: &str, name: &str) -> Option<String> {
    let key = format!(" {name}=\"");
    let start = s.find(&key)? + key.len();
    let len = s.get(start..)?.find('"')?;
    s.get(start..start + len).map(str::to_string)
}

/// Add `project` to the recent-files list (errors are logged by the caller and ignored).
pub fn add(project: &Path, now: &str) -> Result<(), String> {
    add_at(&xbel_path().ok_or("no home folder")?, project, now)
}

/// [`add`] with the bookmark file given. The file keeps its permissions (a new one is private,
/// 0600, as GTK makes it); a symbolic link is refused rather than replaced by a plain file.
pub fn add_at(file: &Path, project: &Path, now: &str) -> Result<(), String> {
    if !project.is_absolute() {
        return Err(format!("not an absolute path: {}", project.display()));
    }
    let (existing, mode) = match std::fs::symlink_metadata(file) {
        Ok(m) if m.file_type().is_symlink() => return Err(format!("{} is a symbolic link; left alone", file.display())),
        Ok(m) if !m.is_file() => return Err(format!("{} is not a file", file.display())),
        Ok(m) if m.len() > MAX_BYTES => return Err(format!("{} is too big to edit", file.display())),
        Ok(m) => (std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?, file_mode(&m)),
        Err(_) => (String::new(), 0o600),
    };
    let uri = crate::desktop::file_uri(project);
    let text = add_bookmark(&existing, &uri, now).ok_or_else(|| format!("{} isn't a bookmark file FilmCraft can edit", file.display()))?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = file.with_extension(format!("xbel.filmcraft{}", std::process::id()));
    let written = write_new(&tmp, text.as_bytes(), mode).and_then(|()| std::fs::rename(&tmp, file));
    written.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", file.display())
    })
}

#[cfg(unix)]
fn file_mode(m: &std::fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode() & 0o777
}
#[cfg(not(unix))]
fn file_mode(_: &std::fs::Metadata) -> u32 {
    0o600
}

/// Create `path` (never an existing file or link) with `mode` and write `bytes`.
fn write_new(path: &Path, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    use std::io::Write;
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(mode);
    }
    #[cfg(not(unix))]
    let _ = mode;
    let mut f = o.open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: &str = "2026-10-12T16:00:00.000000Z";
    const LATER: &str = "2026-10-13T09:30:00.000000Z";

    #[test]
    fn creates_a_new_file() {
        let out = add_bookmark("", "file:///v/My%20Film.fcproj", NOW).unwrap();
        assert!(out.starts_with("<?xml"));
        assert!(out.trim_end().ends_with("</xbel>"));
        assert_eq!(out.matches("<bookmark href=").count(), 1);
        assert!(out.contains("count=\"1\""));
        assert!(out.contains("application/x-filmcraft-project"));
    }

    #[test]
    fn keeps_other_entries_and_updates_ours() {
        let other = "  <bookmark href=\"file:///doc.txt\" added=\"2020-01-01T00:00:00Z\" modified=\"x\" visited=\"x\">\n    <info/>\n  </bookmark>\n";
        let start = format!("{HEADER}{other}</xbel>\n");
        let once = add_bookmark(&start, "file:///v/a.fcproj", NOW).unwrap();
        assert!(once.contains(other), "the other entry is kept byte for byte");
        let twice = add_bookmark(&once, "file:///v/a.fcproj", LATER).unwrap();
        assert!(twice.contains(other));
        assert_eq!(twice.matches("href=\"file:///v/a.fcproj\"").count(), 1, "{twice}");
        assert!(twice.contains(&format!("added=\"{NOW}\"")), "first-added time kept");
        assert!(twice.contains(&format!("modified=\"{LATER}\"")));
        assert!(twice.contains("count=\"2\""));
        // ours moved to the end (newest)
        assert!(twice.find("file:///doc.txt").unwrap() < twice.find("file:///v/a.fcproj").unwrap());
        // a second project is added beside it
        let three = add_bookmark(&twice, "file:///v/b.fcproj", LATER).unwrap();
        assert_eq!(three.matches("<bookmark href=").count(), 3);
    }

    #[test]
    fn gtk_style_hrefs_match_ours() {
        // GTK leaves `(`, `)`, `,` … unescaped; we escape them: still the same file
        let gtk = format!("{HEADER}  <bookmark href=\"file:///v/My%20Film(1),final.fcproj\" added=\"{NOW}\" count=\"3\">\n  </bookmark>\n</xbel>\n");
        let out = add_bookmark(&gtk, "file:///v/My%20Film%281%29%2Cfinal.fcproj", LATER).unwrap();
        assert_eq!(out.matches("<bookmark href=").count(), 1, "{out}");
        assert!(out.contains("count=\"4\""));
        assert_eq!(percent_decode("a%2"), b"a%2", "malformed escapes are kept");
        assert_eq!(percent_decode("%zz%41"), b"%zzA");
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("filmcraft-xbel-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[cfg(unix)]
    #[test]
    fn writes_privately_keeps_modes_and_refuses_links() {
        use std::os::unix::fs::PermissionsExt;
        let d = tmp("perm");
        let file = d.join("share/recently-used.xbel");
        add_at(&file, Path::new("/v/a.fcproj"), NOW).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&file), 0o600, "a new file is private");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
        add_at(&file, Path::new("/v/b.fcproj"), LATER).unwrap();
        assert_eq!(mode(&file), 0o640, "an existing mode is kept");
        let text = std::fs::read_to_string(&file).unwrap();
        assert_eq!(text.matches("<bookmark href=").count(), 2);
        // a symbolic link is left alone
        let target = d.join("elsewhere.xbel");
        std::fs::write(&target, format!("{HEADER}</xbel>\n")).unwrap();
        let link = d.join("link.xbel");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(add_at(&link, Path::new("/v/a.fcproj"), NOW).is_err());
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert!(!std::fs::read_to_string(&target).unwrap().contains("a.fcproj"));
        // too big, a folder, a relative project, a file that isn't xbel: refused, nothing written
        let big = d.join("big.xbel");
        std::fs::File::create(&big).unwrap().set_len(MAX_BYTES + 1).unwrap();
        assert!(add_at(&big, Path::new("/v/a.fcproj"), NOW).is_err());
        assert!(add_at(&d, Path::new("/v/a.fcproj"), NOW).is_err());
        assert!(add_at(&file, Path::new("relative.fcproj"), NOW).is_err());
        let html = d.join("html.xbel");
        std::fs::write(&html, "<html/>").unwrap();
        assert!(add_at(&html, Path::new("/v/a.fcproj"), NOW).is_err());
        assert_eq!(std::fs::read_to_string(&html).unwrap(), "<html/>");
        for dir in [d.clone(), d.join("share")] {
            assert!(!dir.read_dir().unwrap().any(|e| e.unwrap().file_name().to_string_lossy().contains("filmcraft")), "no temp file left");
        }
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn hostile_files_and_uris_are_refused() {
        assert_eq!(add_bookmark("<html>not xbel</html>", "file:///a.fcproj", NOW), None);
        assert_eq!(add_bookmark("", "file:///a\".fcproj", NOW), None);
        assert_eq!(add_bookmark("", "http://example.com/a.fcproj", NOW), None);
        assert_eq!(add_bookmark("", "file:///a.fcproj", "\"><x"), None);
        // our entry opened but never closed: left alone rather than guessed at
        let broken = format!("{HEADER}  <bookmark href=\"file:///a.fcproj\" added=\"x\">\n</xbel>\n");
        assert_eq!(add_bookmark(&broken, "file:///a.fcproj", NOW), None);
        // multibyte text around the entry doesn't break slicing
        let unicode = format!("{HEADER}<!-- 日本語 ü -->\n</xbel>");
        assert!(add_bookmark(&unicode, "file:///a.fcproj", NOW).unwrap().contains("日本語"));
        // a huge count saturates
        let big = format!("{HEADER}  <bookmark href=\"file:///a.fcproj\" added=\"x\" count=\"4294967295\">\n  </bookmark>\n</xbel>\n");
        assert!(add_bookmark(&big, "file:///a.fcproj", NOW).unwrap().contains("count=\"4294967295\""));
    }
}
