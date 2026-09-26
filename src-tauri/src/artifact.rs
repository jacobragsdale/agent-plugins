//! HTTPS artifact download, digest, and safe archive extraction.

use crate::catalog::validate_portable_component;
use crate::locator::{self, sha256_hex};
use crate::sources::{temporary_path, validate_catalog_tree, MAX_SOURCE_BYTES, MAX_SOURCE_FILES};
use flate2::read::GzDecoder;
use reqwest::blocking::{Client, Response};
use reqwest::header::{
    HeaderMap, HeaderValue, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED,
};
use reqwest::redirect::{Action, Attempt, Policy};
use reqwest::StatusCode;
use std::fs::{self, File};
use std::io::{self, Cursor, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

const FETCH_TIMEOUT: Duration = Duration::from_secs(120);
/// An unreachable host fails in seconds instead of holding the sync for the
/// whole download timeout.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REDIRECTS: usize = 5;
const MAX_DOWNLOAD_BYTES: u64 = 50 * 1024 * 1024;
/// Files and folders together; `validate_catalog_tree` holds files to
/// `MAX_SOURCE_FILES` once the archive is out.
const MAX_ARCHIVE_ENTRIES: usize = 2 * MAX_SOURCE_FILES;
/// The most a tar stream may decompress to: the file bytes plus headers,
/// padding, and long names.
const MAX_TAR_STREAM_BYTES: u64 = 2 * MAX_SOURCE_BYTES;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct ArtifactValidators {
    pub(crate) etag: Option<String>,
    pub(crate) last_modified: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DownloadedBytes {
    pub(crate) bytes: Vec<u8>,
    pub(crate) digest: String,
    pub(crate) validators: ArtifactValidators,
}

/// True when the remote artifact is the one the cache holds. An ETag decides
/// on its own; `Last-Modified` decides only when neither side has an ETag.
pub(crate) fn validators_match(stored: &ArtifactValidators, remote: &ArtifactValidators) -> bool {
    match (&stored.etag, &remote.etag) {
        (Some(stored_etag), Some(remote_etag)) => stored_etag == remote_etag,
        (None, None) => stored
            .last_modified
            .as_ref()
            .is_some_and(|stored_modified| remote.last_modified.as_ref() == Some(stored_modified)),
        _ => false,
    }
}

pub(crate) fn head_artifact(url: &str) -> Result<ArtifactValidators, String> {
    let target = fetch_url(url)?;
    let client = client()?;
    let response = crate::marketplace::send(&target, || {
        crate::marketplace::authorize(client.head(&target), &target)
    })?;
    let response = require_success(response, "Could not inspect the artifact")?;
    Ok(validators_from_headers(response.headers()))
}

/// Downloads the artifact. With `cached` validators the request is
/// conditional, and `Ok(None)` means the server answered 304 Not Modified.
pub(crate) fn download_artifact(
    url: &str,
    cached: Option<&ArtifactValidators>,
) -> Result<Option<DownloadedBytes>, String> {
    let target = fetch_url(url)?;
    let client = client()?;
    let response = crate::marketplace::send(&target, || {
        let mut request = client.get(&target);
        if let Some(cached) = cached {
            if let Some(etag) = &cached.etag {
                request = request.header(IF_NONE_MATCH, etag);
            }
            if let Some(modified) = &cached.last_modified {
                request = request.header(IF_MODIFIED_SINCE, modified);
            }
        }
        crate::marketplace::authorize(request, &target)
    })?;
    if cached.is_some() && response.status() == StatusCode::NOT_MODIFIED {
        return Ok(None);
    }
    let response = require_success(response, "Could not download the artifact")?;
    let validators = validators_from_headers(response.headers());
    if let Some(length) = response.content_length() {
        if length > MAX_DOWNLOAD_BYTES {
            return Err("The artifact is larger than the 50 MB download limit.".to_string());
        }
    }
    let mut bytes = Vec::new();
    let mut reader = response;
    let mut buffer = [0_u8; 8192];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("Could not download the artifact: {error}"))?;
        if read == 0 {
            break;
        }
        let next = bytes
            .len()
            .checked_add(read)
            .ok_or_else(|| "The artifact is larger than the 50 MB download limit.".to_string())?;
        if next as u64 > MAX_DOWNLOAD_BYTES {
            return Err("The artifact is larger than the 50 MB download limit.".to_string());
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    let digest = sha256_hex(&bytes);
    Ok(Some(DownloadedBytes {
        bytes,
        digest,
        validators,
    }))
}

pub(crate) fn extract_source_archive(bytes: &[u8], destination: &Path) -> Result<(), String> {
    if looks_like_json(bytes) {
        return Err(
            "This artifact is a JSON document, not a source archive. Add it as a source repository."
                .to_string(),
        );
    }
    fs::create_dir_all(destination)
        .map_err(|error| format!("Could not create {}: {error}", destination.display()))?;
    match detect_archive(bytes)? {
        ArchiveKind::Zip => extract_zip(bytes, destination)?,
        ArchiveKind::Tar => extract_tar(Cursor::new(bytes), destination)?,
        ArchiveKind::TarGz => extract_tar(GzDecoder::new(Cursor::new(bytes)), destination)?,
    }
    unwrap_single_directory(destination)?;
    validate_catalog_tree(destination)
}

pub(crate) fn require_repository_json(bytes: &[u8]) -> Result<(), String> {
    if detect_archive(bytes).is_ok() {
        return Err(
            "This artifact is a source archive, not a source-repository catalog. A source repository artifact must be a JSON document."
                .to_string(),
        );
    }
    if !looks_like_json(bytes) {
        return Err(
            "A source repository artifact must be a JSON document at the HTTPS URL.".to_string(),
        );
    }
    Ok(())
}

fn looks_like_json(bytes: &[u8]) -> bool {
    bytes
        .iter()
        .find(|byte| !byte.is_ascii_whitespace())
        .is_some_and(|byte| *byte == b'{' || *byte == b'[')
}

#[derive(Clone, Copy)]
enum ArchiveKind {
    Zip,
    Tar,
    TarGz,
}

fn detect_archive(bytes: &[u8]) -> Result<ArchiveKind, String> {
    if bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"PK\x05\x06")
        || bytes.starts_with(b"PK\x07\x08")
    {
        return Ok(ArchiveKind::Zip);
    }
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(ArchiveKind::TarGz);
    }
    if bytes.len() > 262 && bytes[257..262] == *b"ustar" {
        return Ok(ArchiveKind::Tar);
    }
    Err("The artifact is not a zip, tar, or tar.gz source archive.".to_string())
}

fn extract_zip(bytes: &[u8], destination: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| format!("Could not read the zip artifact: {error}"))?;
    let mut file_count = 0;
    let mut total_bytes = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| format!("Could not read a zip entry: {error}"))?;
        if entry.is_symlink() {
            return Err("Source archives may not contain symbolic links.".to_string());
        }
        let enclosed = entry
            .enclosed_name()
            .ok_or_else(|| "Source archives may not contain unsafe paths.".to_string())?;
        let relative = sanitize_archive_path(&enclosed)?;
        let path = destination.join(&relative);
        if entry.is_dir() {
            account_extracted_file(0, &mut file_count, &mut total_bytes)?;
            fs::create_dir_all(&path)
                .map_err(|error| format!("Could not create {}: {error}", path.display()))?;
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
        }
        let size = entry.size();
        account_extracted_file(size, &mut file_count, &mut total_bytes)?;
        let mode = entry.unix_mode();
        let mut file = File::create(&path)
            .map_err(|error| format!("Could not create {}: {error}", path.display()))?;
        // The declared size is only checked against the CRC at the end of the
        // entry, so stop one byte past it instead of trusting it.
        let copied = io::copy(&mut (&mut entry).take(size + 1), &mut file)
            .map_err(|error| format!("Could not extract {}: {error}", path.display()))?;
        if copied > size {
            return Err(format!(
                "The zip entry {} is larger than it declares.",
                relative.display()
            ));
        }
        keep_executable_bits(&file, mode, &path)?;
    }
    Ok(())
}

fn extract_tar<R: Read>(reader: R, destination: &Path) -> Result<(), String> {
    let mut archive = tar::Archive::new(reader.take(MAX_TAR_STREAM_BYTES));
    let result = extract_tar_entries(&mut archive, destination);
    // A stream cut off at the bound fails mid-entry or looks like a clean end.
    if archive.into_inner().limit() == 0 {
        return Err("The source expands beyond 50 MB.".to_string());
    }
    result
}

fn extract_tar_entries<R: Read>(
    archive: &mut tar::Archive<R>,
    destination: &Path,
) -> Result<(), String> {
    let mut file_count = 0;
    let mut total_bytes = 0_u64;
    for entry in archive
        .entries()
        .map_err(|error| format!("Could not read the tar artifact: {error}"))?
    {
        let mut entry = entry.map_err(|error| format!("Could not read a tar entry: {error}"))?;
        let header = entry.header();
        let entry_type = header.entry_type();
        if entry_type.is_symlink()
            || entry_type.is_hard_link()
            || entry_type.is_fifo()
            || entry_type.is_character_special()
            || entry_type.is_block_special()
        {
            return Err("Source archives may not contain links or special files.".to_string());
        }
        let entry_path = entry
            .path()
            .map_err(|error| format!("Could not read a tar path: {error}"))?;
        let relative = sanitize_archive_path(&entry_path)?;
        let path = destination.join(&relative);
        if entry_type.is_dir() {
            account_extracted_file(0, &mut file_count, &mut total_bytes)?;
            fs::create_dir_all(&path)
                .map_err(|error| format!("Could not create {}: {error}", path.display()))?;
            continue;
        }
        if !entry_type.is_file()
            && !entry_type.is_gnu_longname()
            && !entry_type.is_pax_local_extensions()
        {
            return Err(format!(
                "Source archives may not contain special entries: {}",
                relative.display()
            ));
        }
        if !entry_type.is_file() {
            continue;
        }
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
        }
        let mode = header.mode().ok();
        // The entry's size, not the header's: a PAX record can override it.
        account_extracted_file(entry.size(), &mut file_count, &mut total_bytes)?;
        let mut file = File::create(&path)
            .map_err(|error| format!("Could not create {}: {error}", path.display()))?;
        io::copy(&mut entry, &mut file)
            .map_err(|error| format!("Could not extract {}: {error}", path.display()))?;
        keep_executable_bits(&file, mode, &path)?;
    }
    Ok(())
}

/// Adds the archive's executable bits to the new file. The rest of the mode
/// stays the process default, so an archive cannot make a file unreadable.
#[cfg(unix)]
fn keep_executable_bits(file: &File, mode: Option<u32>, path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;
    let executable = mode.unwrap_or(0) & 0o111;
    if executable == 0 {
        return Ok(());
    }
    let describe =
        |error: io::Error| format!("Could not set permissions on {}: {error}", path.display());
    let mut permissions = file.metadata().map_err(describe)?.permissions();
    permissions.set_mode(permissions.mode() | executable);
    file.set_permissions(permissions).map_err(describe)
}

#[cfg(not(unix))]
fn keep_executable_bits(_file: &File, _mode: Option<u32>, _path: &Path) -> Result<(), String> {
    Ok(())
}

fn sanitize_archive_path(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        return Err("Source archives may not contain absolute paths.".to_string());
    }
    let mut sanitized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(name) => {
                validate_portable_component(name, path)?;
                sanitized.push(name);
            }
            Component::CurDir => {}
            Component::ParentDir => {
                return Err("Source archives may not contain parent-directory paths.".to_string());
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err("Source archives may not contain absolute paths.".to_string());
            }
        }
    }
    if sanitized.as_os_str().is_empty() {
        return Err("Source archives may not contain empty paths.".to_string());
    }
    Ok(sanitized)
}

fn account_extracted_file(
    size: u64,
    file_count: &mut usize,
    total_bytes: &mut u64,
) -> Result<(), String> {
    *file_count = file_count
        .checked_add(1)
        .ok_or_else(|| "The source contains too many files.".to_string())?;
    if *file_count > MAX_ARCHIVE_ENTRIES {
        return Err(format!(
            "The source archive contains more than {MAX_ARCHIVE_ENTRIES} files and folders."
        ));
    }
    *total_bytes = total_bytes
        .checked_add(size)
        .ok_or_else(|| "The source is too large.".to_string())?;
    if *total_bytes > MAX_SOURCE_BYTES {
        return Err("The source expands beyond 50 MB.".to_string());
    }
    Ok(())
}

fn unwrap_single_directory(root: &Path) -> Result<(), String> {
    let mut files = Vec::new();
    let mut directories = Vec::new();
    for entry in fs::read_dir(root)
        .map_err(|error| format!("Could not inspect {}: {error}", root.display()))?
    {
        let entry =
            entry.map_err(|error| format!("Could not inspect {}: {error}", root.display()))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("Could not inspect {}: {error}", entry.path().display()))?;
        if file_type.is_dir() {
            directories.push(entry.path());
        } else if file_type.is_file() {
            files.push(entry.path());
        } else {
            return Err(format!(
                "Source entry is not a regular file or directory: {}",
                entry.path().display()
            ));
        }
    }
    if files.is_empty() && directories.len() == 1 {
        let only = directories
            .into_iter()
            .next()
            .expect("one top-level directory");
        let staging = temporary_path(root, "unwrap");
        fs::rename(&only, &staging)
            .map_err(|error| format!("Could not unwrap {}: {error}", only.display()))?;
        for entry in fs::read_dir(&staging)
            .map_err(|error| format!("Could not unwrap {}: {error}", staging.display()))?
        {
            let entry = entry
                .map_err(|error| format!("Could not unwrap {}: {error}", staging.display()))?;
            let destination = root.join(entry.file_name());
            fs::rename(entry.path(), &destination).map_err(|error| {
                format!(
                    "Could not unwrap {} to {}: {error}",
                    entry.path().display(),
                    destination.display()
                )
            })?;
        }
        fs::remove_dir(&staging)
            .map_err(|error| format!("Could not unwrap {}: {error}", staging.display()))?;
    }
    Ok(())
}

pub(crate) static CLIENT: Mutex<Option<Client>> = Mutex::new(None);

fn client() -> Result<Client, String> {
    crate::marketplace::cached_client(&CLIENT, |builder| {
        builder
            .timeout(FETCH_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .redirect(Policy::custom(redirect_policy))
            .build()
    })
}

fn redirect_policy(attempt: Attempt<'_>) -> Action {
    if attempt.previous().len() >= MAX_REDIRECTS {
        return attempt.error("The artifact URL redirected more than 5 times.");
    }
    match allowed_redirect_url(attempt.url().as_str()) {
        Ok(()) => attempt.follow(),
        Err(error) => attempt.error(error),
    }
}

fn allowed_redirect_url(url: &str) -> Result<(), String> {
    #[cfg(test)]
    {
        if is_loopback_http(url) {
            return Ok(());
        }
    }
    locator::canonicalize_artifact_url(url).map(|_| ())
}

fn fetch_url(url: &str) -> Result<String, String> {
    #[cfg(test)]
    {
        if is_loopback_http(url) {
            return Ok(url.to_string());
        }
    }
    locator::canonicalize_artifact_url(url)
}

#[cfg(test)]
fn is_loopback_http(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|parsed| {
        parsed.scheme() == "http"
            && parsed.username().is_empty()
            && parsed.password().is_none()
            && parsed
                .host_str()
                .is_some_and(|host| host == "127.0.0.1" || host == "localhost" || host == "[::1]")
    })
}

/// True when a refresh error means the artifact no longer exists at its URL
/// (HTTP 404 or 410), as opposed to a transient failure.
pub(crate) fn is_gone(message: &str) -> bool {
    message.contains("HTTP 404") || message.contains("HTTP 410")
}

/// True when the request never reached the server: the connection was
/// refused, the name did not resolve, the connect timed out, or Windows could
/// not reach a domain controller to sign the request.
pub(crate) fn is_connect_failure(message: &str) -> bool {
    message.contains(crate::marketplace::CONNECT_FAILURE)
        || message.contains(crate::host_identity::NO_DOMAIN_CONTROLLER)
}

/// True when the server was not reached or did not answer in time.
pub(crate) fn is_unreachable(message: &str) -> bool {
    is_connect_failure(message) || message.contains(crate::marketplace::TIMED_OUT)
}

/// True when the failure is worth trying again later: the server was not
/// reached, was too slow, was overloaded, or asked the client to back off.
pub(crate) fn is_transient(message: &str) -> bool {
    is_unreachable(message) || server_busy(message)
}

/// True when the server answered, but was failing or asked the client to back
/// off. The machine is online, so this is not reported as being offline.
pub(crate) fn server_busy(message: &str) -> bool {
    message.contains("(HTTP 5") || message.contains("(HTTP 429")
}

/// The status code stays in the text (`is_gone` and `is_transient` read it);
/// the server's problem title or the bare status follows it.
fn require_success(response: Response, operation: &str) -> Result<Response, String> {
    let status = response.status();
    if status.is_success() {
        return Ok(response);
    }
    let url = response.url().to_string();
    Err(format!(
        "{operation} (HTTP {}): {}",
        status.as_u16(),
        crate::marketplace::failure(&url, response)
    ))
}

fn validators_from_headers(headers: &HeaderMap) -> ArtifactValidators {
    ArtifactValidators {
        etag: header_text(headers.get(ETAG)),
        last_modified: header_text(headers.get(LAST_MODIFIED)),
    }
}

fn header_text(value: Option<&HeaderValue>) -> Option<String> {
    value
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn source_tree() -> (tempfile::TempDir, PathBuf) {
        let root = tempfile::tempdir().expect("tree");
        let skill = root.path().join("skills/review");
        fs::create_dir_all(&skill).expect("skill");
        fs::write(
            skill.join("SKILL.md"),
            "---\nname: review\ndescription: Reviews code\n---\nBody\n",
        )
        .expect("skill");
        fs::write(
            root.path().join("agent-plugins.json"),
            r#"{
              "version": 2,
              "source": { "id": "acme", "name": "Acme", "description": "Test source" },
              "packages": [{
                "id": "review",
                "components": [{"kind": "skill", "path": "skills/review"}]
              }]
            }"#,
        )
        .expect("manifest");
        (root, skill)
    }

    fn zip_bytes(root: &Path, prefix: Option<&str>) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut cursor);
            let options =
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            add_zip_tree(&mut zip, root, root, prefix, options);
            zip.finish().expect("zip");
        }
        cursor.into_inner()
    }

    fn add_zip_tree(
        zip: &mut ZipWriter<&mut Cursor<Vec<u8>>>,
        root: &Path,
        directory: &Path,
        prefix: Option<&str>,
        options: SimpleFileOptions,
    ) {
        let mut entries = fs::read_dir(directory)
            .expect("read")
            .collect::<Result<Vec<_>, _>>()
            .expect("entries");
        entries.sort_by_key(fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let relative = path.strip_prefix(root).expect("relative");
            let name = match prefix {
                Some(prefix) => Path::new(prefix).join(relative),
                None => relative.to_path_buf(),
            };
            let name = name.to_str().expect("utf-8").replace('\\', "/");
            if path.is_dir() {
                zip.add_directory(format!("{name}/"), options).expect("dir");
                add_zip_tree(zip, root, &path, prefix, options);
            } else {
                zip.start_file(name, options).expect("file");
                zip.write_all(&fs::read(&path).expect("read file"))
                    .expect("write");
            }
        }
    }

    fn zip_with_name(name: &str) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut cursor);
            zip.start_file(
                name,
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
            )
            .expect("start");
            zip.write_all(b"nope").expect("write");
            zip.finish().expect("finish");
        }
        cursor.into_inner()
    }

    #[test]
    fn zip_slip_is_rejected() {
        let destination = tempfile::tempdir().expect("dest");
        let error = extract_source_archive(&zip_with_name("../evil.txt"), destination.path())
            .expect_err("zip-slip");
        assert!(
            error.contains("unsafe") || error.contains("parent-directory"),
            "{error}"
        );
        assert!(!destination.path().join("evil.txt").exists());
    }

    #[test]
    fn zip_parent_paths_are_rejected() {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut cursor);
            zip.start_file(
                "ok/../../evil.txt",
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
            )
            .expect("start");
            zip.write_all(b"nope").expect("write");
            zip.finish().expect("finish");
        }
        let destination = tempfile::tempdir().expect("dest");
        let error =
            extract_source_archive(&cursor.into_inner(), destination.path()).expect_err("parent");
        assert!(
            error.contains("unsafe") || error.contains("parent-directory"),
            "{error}"
        );
        assert!(!destination.path().join("evil.txt").exists());
    }

    #[test]
    fn single_directory_archives_unwrap() {
        let (tree, _) = source_tree();
        let bytes = zip_bytes(tree.path(), Some("repo-main"));
        let destination = tempfile::tempdir().expect("dest");
        extract_source_archive(&bytes, destination.path()).expect("extract");
        assert!(destination.path().join("agent-plugins.json").is_file());
        assert!(destination.path().join("skills/review/SKILL.md").is_file());
        assert!(!destination.path().join("repo-main").exists());
    }

    #[test]
    fn zip_entries_cannot_inflate_past_their_declared_size() {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut cursor);
            zip.start_file(
                "big.txt",
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
            )
            .expect("start");
            zip.write_all(&vec![0; 8 * 1024 * 1024]).expect("write");
            zip.finish().expect("finish");
        }
        let mut bytes = cursor.into_inner();
        // Declare one byte in the local header and the central directory.
        bytes[22..26].copy_from_slice(&1_u32.to_le_bytes());
        let central = bytes
            .windows(4)
            .position(|window| window == b"PK\x01\x02")
            .expect("central directory");
        bytes[central + 24..central + 28].copy_from_slice(&1_u32.to_le_bytes());
        let destination = tempfile::tempdir().expect("dest");
        let error = extract_source_archive(&bytes, destination.path()).expect_err("bomb");
        assert!(error.contains("larger than it declares"), "{error}");
        let written = fs::metadata(destination.path().join("big.txt")).map_or(0, |m| m.len());
        assert!(written <= 2, "{written} bytes written");
    }

    #[test]
    fn tar_gz_streams_stop_at_the_bound() {
        let mut header = tar::Header::new_gnu();
        header.set_path("././@LongLink").expect("path");
        header.set_entry_type(tar::EntryType::GNULongName);
        header.set_size(MAX_TAR_STREAM_BYTES + 1);
        header.set_cksum();
        // Repeat one sync-flushed block of zeros instead of deflating the
        // whole stream, which is slow in a debug build.
        let mut compress = flate2::Compress::new(flate2::Compression::fast(), false);
        let mut deflate = |input: &[u8]| {
            let mut output = Vec::with_capacity(input.len() + 1024);
            compress
                .compress_vec(input, &mut output, flate2::FlushCompress::Sync)
                .expect("deflate");
            output
        };
        let mut bytes = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
        bytes.extend(deflate(header.as_bytes()));
        let chunk = 1024 * 1024;
        let zeros = deflate(&vec![0; chunk]);
        for _ in 0..=MAX_TAR_STREAM_BYTES / chunk as u64 {
            bytes.extend_from_slice(&zeros);
        }
        let destination = tempfile::tempdir().expect("dest");
        let error = extract_source_archive(&bytes, destination.path()).expect_err("bomb");
        assert!(error.contains("beyond 50 MB"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn extraction_keeps_executable_bits() {
        use std::os::unix::fs::PermissionsExt as _;
        let (tree, skill) = source_tree();
        let script = skill.join("x.sh");
        fs::write(&script, "#!/bin/sh\n").expect("script");
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).expect("chmod");
        let zip = crate::staging::zip_tree(tree.path()).expect("zip");
        let mut builder = tar::Builder::new(Vec::new());
        builder.mode(tar::HeaderMode::Complete);
        builder.append_dir_all("repo", tree.path()).expect("tar");
        let tar = builder.into_inner().expect("tar");
        for bytes in [zip, tar] {
            let destination = tempfile::tempdir().expect("dest");
            extract_source_archive(&bytes, destination.path()).expect("extract");
            let mode = fs::metadata(destination.path().join("skills/review/x.sh"))
                .expect("x.sh")
                .permissions()
                .mode();
            assert_ne!(mode & 0o111, 0, "{mode:o}");
            let plain = fs::metadata(destination.path().join("agent-plugins.json"))
                .expect("manifest")
                .permissions()
                .mode();
            assert_eq!(plain & 0o111, 0, "{plain:o}");
        }
    }

    #[test]
    fn json_payload_is_rejected_as_a_source_archive() {
        let destination = tempfile::tempdir().expect("dest");
        assert!(
            extract_source_archive(br#"{"version":1}"#, destination.path())
                .expect_err("json")
                .contains("source repository")
        );
    }

    struct Recorded {
        gets: usize,
        heads: usize,
    }

    fn serve_fixture(
        body: Vec<u8>,
        etag: Option<&'static str>,
        last_modified: Option<&'static str>,
    ) -> (String, Arc<Mutex<Recorded>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("addr");
        let recorded = Arc::new(Mutex::new(Recorded { gets: 0, heads: 0 }));
        let counts = Arc::clone(&recorded);
        let handle = thread::spawn(move || {
            listener.set_nonblocking(false).expect("blocking");
            for _ in 0..8 {
                let Ok((stream, _)) = listener.accept() else {
                    break;
                };
                let mut reader = BufReader::new(stream);
                let mut request = String::new();
                if reader.read_line(&mut request).is_err() {
                    continue;
                }
                let mut line = String::new();
                let mut if_none_match = None;
                while reader.read_line(&mut line).is_ok() {
                    if line == "\r\n" || line == "\n" {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':') {
                        if name.eq_ignore_ascii_case("if-none-match") {
                            if_none_match = Some(value.trim().to_string());
                        }
                    }
                    line.clear();
                }
                let method = request.split_whitespace().next().unwrap_or("");
                {
                    let mut counts = counts.lock().expect("lock");
                    if method == "HEAD" {
                        counts.heads += 1;
                    } else if method == "GET" {
                        counts.gets += 1;
                    }
                }
                let mut stream = reader.into_inner();
                if etag.is_some() && if_none_match.as_deref() == etag {
                    let _ = stream.write_all(
                        b"HTTP/1.1 304 Not Modified\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                    continue;
                }
                let mut headers = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n",
                    body.len()
                );
                if let Some(etag) = etag {
                    headers.push_str(&format!("ETag: {etag}\r\n"));
                }
                if let Some(last_modified) = last_modified {
                    headers.push_str(&format!("Last-Modified: {last_modified}\r\n"));
                }
                headers.push_str("\r\n");
                let _ = stream.write_all(headers.as_bytes());
                if method == "GET" {
                    let _ = stream.write_all(&body);
                }
                let _ = stream.flush();
            }
        });
        (
            format!("http://127.0.0.1:{}/source.zip", address.port()),
            recorded,
            handle,
        )
    }

    #[test]
    fn digest_changes_when_payload_changes() {
        let (tree, _) = source_tree();
        let first = zip_bytes(tree.path(), None);
        fs::write(tree.path().join("extra.txt"), "changed").expect("change");
        let second = zip_bytes(tree.path(), None);
        let (url, _, _) = serve_fixture(first.clone(), None, None);
        let downloaded = download_artifact(&url, None)
            .expect("download")
            .expect("body");
        assert_eq!(downloaded.digest, sha256_hex(&first));
        assert_ne!(downloaded.digest, sha256_hex(&second));
    }

    #[test]
    fn etag_and_last_modified_short_circuit() {
        let (tree, _) = source_tree();
        let body = zip_bytes(tree.path(), None);
        let (url, counts, _) =
            serve_fixture(body, Some("\"abc\""), Some("Wed, 21 Oct 2015 07:28:00 GMT"));
        let first = download_artifact(&url, None).expect("get").expect("body");
        let head = head_artifact(&url).expect("head");
        assert!(validators_match(&first.validators, &head));
        assert_eq!(
            download_artifact(&url, Some(&first.validators)).expect("conditional"),
            None,
            "a matching If-None-Match is answered with 304"
        );
        let recorded = counts.lock().expect("lock");
        assert_eq!(recorded.gets, 2);
        assert_eq!(recorded.heads, 1);
    }

    #[test]
    fn an_etag_alone_decides_a_match() {
        let etag = |value: &str| ArtifactValidators {
            etag: Some(value.to_string()),
            last_modified: None,
        };
        assert!(validators_match(&etag("\"a\""), &etag("\"a\"")));
        assert!(!validators_match(&etag("\"a\""), &etag("\"b\"")));
        assert!(!validators_match(
            &ArtifactValidators::default(),
            &ArtifactValidators::default()
        ));
    }

    #[test]
    fn overloaded_servers_are_retried_after_retry_after() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let address = listener.local_addr().expect("addr");
        thread::spawn(move || {
            for status in [
                "503 Service Unavailable\r\nRetry-After: 0",
                "200 OK\r\nETag: \"v\"",
            ] {
                let Ok((stream, _)) = listener.accept() else {
                    return;
                };
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                while reader.read_line(&mut line).is_ok_and(|read| read > 2) {
                    line.clear();
                }
                let _ = reader.into_inner().write_all(
                    format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .as_bytes(),
                );
            }
        });
        let head = head_artifact(&format!("http://{address}/source.zip")).expect("retried");
        assert_eq!(head.etag.as_deref(), Some("\"v\""));
    }

    #[test]
    fn failures_keep_the_status_for_classification() {
        let closed = TcpListener::bind("127.0.0.1:0").expect("bind");
        let dead = format!("http://{}/gone.zip", closed.local_addr().expect("addr"));
        drop(closed);
        let error = head_artifact(&dead).expect_err("refused");
        assert!(is_connect_failure(&error), "{error}");
        assert!(is_transient(&error));
        assert!(is_gone("Could not download the artifact (HTTP 404): x"));
        assert!(!is_transient(
            "Could not download the artifact (HTTP 404): x"
        ));
        assert!(is_transient(
            "Could not download the artifact (HTTP 503): x"
        ));
    }

    #[test]
    fn requests_share_one_client_until_reset() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://{}/source.zip", listener.local_addr().expect("addr"));
        let connections = Arc::new(Mutex::new(0));
        let accepted = Arc::clone(&connections);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                *accepted.lock().expect("lock") += 1;
                thread::spawn(move || {
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    loop {
                        line.clear();
                        match reader.read_line(&mut line) {
                            Ok(0) | Err(_) => return,
                            Ok(_) if line == "\r\n" => {
                                let _ = reader
                                    .get_mut()
                                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n");
                            }
                            Ok(_) => {}
                        }
                    }
                });
            }
        });
        head_artifact(&url).expect("first");
        head_artifact(&url).expect("second");
        assert_eq!(
            *connections.lock().expect("lock"),
            1,
            "one pooled connection"
        );
        crate::marketplace::reset_http_clients();
        head_artifact(&url).expect("after reset");
        assert_eq!(*connections.lock().expect("lock"), 2, "a fresh client");
    }

    #[test]
    fn an_unreachable_domain_controller_is_offline() {
        let error = format!(
            "{} (Windows code 0x80090311 for HTTP/marketplace.example.com)",
            crate::host_identity::NO_DOMAIN_CONTROLLER
        );
        assert!(is_connect_failure(&error));
        assert!(is_unreachable(&error));
        assert_eq!(
            crate::ipc_error::IpcError::from(error).kind,
            crate::ipc_error::IpcErrorKind::Offline
        );
    }
}
