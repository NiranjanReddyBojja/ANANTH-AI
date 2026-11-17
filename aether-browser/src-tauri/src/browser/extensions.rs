//! Chrome extension support: discovering unpacked extensions, unpacking
//! `.crx` packages, and producing the switches Chromium needs to load them.

use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ExtensionInfo {
    pub id: String,
    pub name: String,
    pub version: String,
    pub manifest_version: u64,
    pub description: String,
    pub path: PathBuf,
}

/// Reads the manifest of an unpacked extension directory.
pub fn read_extension(dir: &Path) -> Result<ExtensionInfo> {
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("manifest.json"))?)
        .map_err(|error| Error::Invalid(format!("manifest.json: {error}")))?;
    let text = |key: &str| manifest.get(key).and_then(|value| value.as_str()).unwrap_or_default().to_owned();
    let manifest_version = manifest.get("manifest_version").and_then(|value| value.as_u64()).unwrap_or(0);
    if !(2..=3).contains(&manifest_version) {
        return Err(Error::Invalid("unsupported manifest_version".into()));
    }
    let name = text("name");
    if name.is_empty() {
        return Err(Error::Invalid("extension has no name".into()));
    }
    Ok(ExtensionInfo {
        id: dir.file_name().unwrap_or_default().to_string_lossy().into_owned(),
        name,
        version: text("version"),
        manifest_version,
        description: text("description"),
        path: dir.to_path_buf(),
    })
}

/// Lists the valid unpacked extensions inside `extensions_dir`.
pub fn scan_extensions(extensions_dir: &Path) -> Vec<ExtensionInfo> {
    let Ok(entries) = std::fs::read_dir(extensions_dir) else {
        return Vec::new();
    };
    let mut found: Vec<ExtensionInfo> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| read_extension(&entry.path()).ok())
        .collect();
    found.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    found
}

/// Value for Chromium's `--load-extension` switch.
pub fn load_extension_switch(extensions: &[ExtensionInfo]) -> Option<String> {
    if extensions.is_empty() {
        return None;
    }
    Some(extensions.iter().map(|ext| ext.path.to_string_lossy().into_owned()).collect::<Vec<_>>().join(","))
}

/// Returns the ZIP archive inside a CRX2 or CRX3 package.
pub fn crx_zip_payload(crx: &[u8]) -> Result<&[u8]> {
    let invalid = || Error::Invalid("not a valid .crx file".into());
    let u32_at = |offset: usize| -> Result<usize> {
        crx.get(offset..offset + 4)
            .map(|bytes| u32::from_le_bytes(bytes.try_into().unwrap()) as usize)
            .ok_or_else(invalid)
    };
    if crx.get(..4) != Some(b"Cr24") {
        // Some stores serve a plain ZIP.
        return if crx.starts_with(b"PK\x03\x04") { Ok(crx) } else { Err(invalid()) };
    }
    let start = match u32_at(4)? {
        2 => 16usize.checked_add(u32_at(8)?).and_then(|n| n.checked_add(u32_at(12).ok()?)),
        3 => 12usize.checked_add(u32_at(8)?),
        _ => None,
    }
    .ok_or_else(invalid)?;
    crx.get(start..).filter(|zip| zip.starts_with(b"PK\x03\x04")).ok_or_else(invalid)
}

/// Unpacks a `.crx` (or ZIP) extension into `dest`, which is created.
pub fn unpack_crx(crx: &[u8], dest: &Path) -> Result<ExtensionInfo> {
    const MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
    let mut archive = zip::ZipArchive::new(Cursor::new(crx_zip_payload(crx)?))
        .map_err(|error| Error::Invalid(format!("extension archive: {error}")))?;
    std::fs::create_dir_all(dest)?;
    let mut total = 0u64;
    for index in 0..archive.len() {
        let mut file = archive.by_index(index).map_err(|error| Error::Invalid(error.to_string()))?;
        // Reject absolute paths and `..` so nothing is written outside `dest`.
        let relative = file
            .enclosed_name()
            .filter(|path| path.components().all(|part| matches!(part, Component::Normal(_))))
            .ok_or_else(|| Error::Invalid("extension archive contains an unsafe path".into()))?;
        let target = dest.join(relative);
        if file.is_dir() {
            std::fs::create_dir_all(&target)?;
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = std::fs::File::create(&target)?;
        let written = std::io::copy(&mut (&mut file).take(MAX_TOTAL_BYTES - total + 1), &mut out)?;
        total += written;
        if total > MAX_TOTAL_BYTES {
            return Err(Error::Invalid("extension is too large".into()));
        }
    }
    read_extension(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with(files: &[(&str, &str)]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, body) in files {
            writer.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(body.as_bytes()).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    const MANIFEST: &str = r#"{"manifest_version":3,"name":"Dark Reader","version":"4.9","description":"Dark mode"}"#;

    fn crx3(zip: &[u8]) -> Vec<u8> {
        let header = b"fake-signed-header";
        let mut crx = b"Cr24".to_vec();
        crx.extend_from_slice(&3u32.to_le_bytes());
        crx.extend_from_slice(&(header.len() as u32).to_le_bytes());
        crx.extend_from_slice(header);
        crx.extend_from_slice(zip);
        crx
    }

    #[test]
    fn unpacks_crx3_and_reads_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let crx = crx3(&zip_with(&[("manifest.json", MANIFEST), ("js/content.js", "console.log(1)")]));
        let info = unpack_crx(&crx, &dir.path().join("darkreader")).unwrap();
        assert_eq!((info.name.as_str(), info.version.as_str(), info.manifest_version), ("Dark Reader", "4.9", 3));
        assert!(dir.path().join("darkreader/js/content.js").is_file());

        let scanned = scan_extensions(dir.path());
        assert_eq!(scanned.len(), 1);
        assert!(load_extension_switch(&scanned).unwrap().ends_with("darkreader"));
        assert_eq!(load_extension_switch(&[]), None);
    }

    #[test]
    fn crx2_and_plain_zip_payloads() {
        let zip = zip_with(&[("manifest.json", MANIFEST)]);
        let mut crx2 = b"Cr24".to_vec();
        crx2.extend_from_slice(&2u32.to_le_bytes());
        crx2.extend_from_slice(&3u32.to_le_bytes());
        crx2.extend_from_slice(&2u32.to_le_bytes());
        crx2.extend_from_slice(b"keysg");
        crx2.extend_from_slice(&zip);
        assert_eq!(crx_zip_payload(&crx2).unwrap(), zip.as_slice());
        assert_eq!(crx_zip_payload(&zip).unwrap(), zip.as_slice());
    }

    #[test]
    fn rejects_garbage_truncation_and_bad_manifests() {
        assert!(crx_zip_payload(b"hello world").is_err());
        assert!(crx_zip_payload(b"Cr24\x03\x00\x00\x00\xff\xff\xff\x7f").is_err());
        assert!(crx_zip_payload(b"Cr24\x09\x00\x00\x00\x00\x00\x00\x00").is_err());

        let dir = tempfile::tempdir().unwrap();
        let no_name = crx3(&zip_with(&[("manifest.json", r#"{"manifest_version":3}"#)]));
        assert!(unpack_crx(&no_name, &dir.path().join("a")).is_err());
        let old = crx3(&zip_with(&[("manifest.json", r#"{"manifest_version":1,"name":"x"}"#)]));
        assert!(unpack_crx(&old, &dir.path().join("b")).is_err());
    }

    #[test]
    fn refuses_paths_that_escape_the_destination() {
        let dir = tempfile::tempdir().unwrap();
        let evil = crx3(&zip_with(&[("manifest.json", MANIFEST), ("../escaped.txt", "x")]));
        assert!(unpack_crx(&evil, &dir.path().join("ext")).is_err());
        assert!(!dir.path().join("escaped.txt").exists());
    }
}
