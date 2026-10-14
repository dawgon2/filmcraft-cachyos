//! The downloadable DeepFilterNet3 package: the official `DeepFilterNet3.zip` (a PyTorch
//! checkpoint and its `config.ini`), pinned to a commit of the DeepFilterNet repository and checked
//! against its SHA-256 by the downloader; the checkpoint is then extracted next to it. Never
//! bundled or committed.

use std::io::{Read, Write};
use std::path::Path;

use crate::ModelError;
use crate::weights::MAX_FILE_BYTES;

/// Folder under the models directory.
pub const DIR: &str = "deepfilternet3";
pub const NAME: &str = "DeepFilterNet3 (voice isolation)";
pub const ZIP_NAME: &str = "DeepFilterNet3.zip";
pub const ZIP_URL: &str = "https://raw.githubusercontent.com/Rikorose/DeepFilterNet/82b0c7ad33fc756284104053817d1e855d8d8386/models/DeepFilterNet3.zip";
pub const ZIP_SHA256: &str = "49c52edc8947ae1f9bf50d81530beaf3a2c3245aeaf34b6f31ff535cd22284d2";
pub const ZIP_SIZE: u64 = 7_986_207;
/// The extracted checkpoint the effect loads.
pub const CHECKPOINT: &str = "model_120.ckpt.best";
pub const LICENSE: &str = "MIT OR Apache-2.0";
pub const LICENSE_URL: &str = "https://github.com/Rikorose/DeepFilterNet/blob/main/LICENSE";
pub const AUTHOR: &str = "Hendrik Schröter et al. (DeepFilterNet)";
pub const SOURCE: &str = "https://github.com/Rikorose/DeepFilterNet";

/// Copy the checkpoint out of the release zip into `dest` (via `<dest>.part`, renamed when
/// complete). The zip's own checksum is the downloader's job; the checkpoint is checked when it
/// loads.
pub fn extract_checkpoint(zip_path: &Path, dest: &Path) -> Result<(), ModelError> {
    let err = |e: &dyn std::fmt::Display| ModelError(format!("{}: {e}", zip_path.display()));
    let file = std::fs::File::open(zip_path).map_err(|e| err(&e))?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| err(&e))?;
    let name = zip
        .file_names()
        .find(|n| n.ends_with(&format!("checkpoints/{CHECKPOINT}")))
        .map(str::to_string)
        .ok_or_else(|| err(&"no DeepFilterNet3 checkpoint in this zip"))?;
    let entry = zip.by_name(&name).map_err(|e| err(&e))?;
    if entry.size() > MAX_FILE_BYTES {
        return Err(err(&"the checkpoint is too large"));
    }
    let part = dest.with_extension("part");
    let mut out = std::fs::File::create(&part).map_err(|e| err(&e))?;
    let mut buf = Vec::new();
    entry.take(MAX_FILE_BYTES).read_to_end(&mut buf).map_err(|e| err(&e))?;
    out.write_all(&buf).and_then(|()| out.flush()).map_err(|e| err(&e))?;
    drop(out);
    std::fs::rename(&part, dest).map_err(|e| err(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_zips_are_errors() {
        let dir = std::env::temp_dir().join(format!("fc-voice-pkg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join(CHECKPOINT);
        assert!(extract_checkpoint(&dir.join("absent.zip"), &dest).is_err());
        let junk = dir.join("junk.zip");
        std::fs::write(&junk, b"PK\x03\x04 not really a zip").unwrap();
        assert!(extract_checkpoint(&junk, &dest).is_err());
        // A real zip without the checkpoint.
        let other = dir.join("other.zip");
        {
            let mut z = zip::ZipWriter::new(std::fs::File::create(&other).unwrap());
            z.start_file("DeepFilterNet3/config.ini", zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(b"[df]\n").unwrap();
            z.finish().unwrap();
        }
        assert!(extract_checkpoint(&other, &dest).unwrap_err().0.contains("no DeepFilterNet3 checkpoint"));
        assert!(!dest.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// With `FILMCRAFT_DFN3_ZIP` (the release zip): the checkpoint comes out and loads.
    #[test]
    fn release_zip_when_present() {
        let Some(zip) = std::env::var_os("FILMCRAFT_DFN3_ZIP") else {
            eprintln!("FILMCRAFT_DFN3_ZIP unset: skipped");
            return;
        };
        let dir = std::env::temp_dir().join(format!("fc-voice-pkg-real-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join(CHECKPOINT);
        extract_checkpoint(Path::new(&zip), &dest).unwrap();
        crate::weights::Weights::load(&dest).unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }
}
