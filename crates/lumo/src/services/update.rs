//! Reads stable releases from Lumo's own GitHub API over HTTPS. A requested
//! worker exists only for the check/download; it keeps an installer only
//! when its size and GitHub-provided SHA-256 match. `install` rechecks it.
//!

use crate::events::{AppEvent, EventSink};
use crate::services::addons::Job;
use crate::services::http::Client;
use lumo_core::update::{self, Release};
use std::time::Duration;

/// Budget for the file: it is small, but a slow line still gets it.
const TIMEOUT: Duration = Duration::from_secs(20);
/// Budget for the Setup (about 2.5 MB): a slow line's few minutes.
#[cfg(windows)]
const SETUP_TIMEOUT: Duration = Duration::from_secs(300);

/// What the update file said.
#[derive(Clone, Debug, PartialEq)]
pub enum UpdateEvent {
    /// Latest stable release with GitHub-verified installer metadata.
    Checked(Result<Option<Release>, String>),
    /// The new version's Setup is on disk and checked, or why not.
    #[cfg(windows)]
    Downloaded(Result<std::path::PathBuf, String>),
}

/// A job for the add-on thread: read the file, answer with `AppEvent::Update`.
pub fn check_job(sink: EventSink) -> Job {
    Box::new(move |client: Result<&Client, &str>| {
        let r = client.map_err(str::to_string).and_then(check);
        match &r {
            Ok(Some(rel)) => crate::pinfo!("updates: the latest release is {} ({})", rel.version, rel.date),
            Ok(None) => crate::pinfo!("updates: no published Windows update"),
            Err(e) => crate::pinfo!("updates: couldn't check releases ({e})"),
        }
        sink.send(AppEvent::Update(UpdateEvent::Checked(r)));
    })
}

fn check(client: &Client) -> Result<Option<Release>, String> {
    if !update::ENABLED { return Err("Updates are temporarily disabled".into()); }
    let (status, body) = client.get_bytes(update::RELEASES_URL, TIMEOUT, update::MAX_RELEASE_BYTES, |_| {}).map_err(|e| e.to_string())?;
    if status != 200 {
        return Err(if status == 403 || status == 429 { "GitHub is limiting requests. Please try again later.".into() } else { format!("Couldn't check for updates (HTTP {status}). Try again.") });
    }
    update::parse_releases(std::str::from_utf8(&body).map_err(|_| "GitHub returned an unreadable response".to_string())?)
}

/// The file's text to the release it describes, once its signature checks out.
#[cfg(test)]
pub fn believe(text: &str) -> Result<Release, String> {
    let signed = update::parse_file(text)?;
    if !verify(signed.payload.as_bytes(), &signed.signature, &signed.key) {
        return Err("its signature doesn't match, so it was ignored".into());
    }
    update::parse_payload(&signed.payload)
}

/// A job for the add-on thread: put the Setup `d` at `path`, unless it is
/// there already, and answer with `UpdateEvent::Downloaded`.
#[cfg(windows)]
pub fn download_job(sink: EventSink, d: update::Download, path: std::path::PathBuf) -> Job {
    Box::new(move |client: Result<&Client, &str>| {
        let r = download(client, &d, &path);
        match &r {
            Ok(()) => crate::pinfo!("updates: {} is ready", d.file_name()),
            Err(e) => crate::pinfo!("updates: couldn't download {} ({e})", d.file_name()),
        }
        sink.send(AppEvent::Update(UpdateEvent::Downloaded(r.map(|()| path))));
    })
}

#[cfg(windows)]
fn download(client: Result<&Client, &str>, d: &update::Download, path: &std::path::Path) -> Result<(), String> {
    if !update::ENABLED { return Err("Updates are temporarily disabled".into()); }
    if !d.allowed() || path.file_name().is_none_or(|name| name != d.file_name()) { return Err("Untrusted update installer".into()); }
    use std::fs;
    // Downloaded on an earlier run: checked again, no request.
    if fs::read(path).is_ok_and(|b| d.matches(&b)) {
        return Ok(());
    }
    let (status, bytes) = client?.get_bytes(&d.url, SETUP_TIMEOUT, d.size + 64 * 1024, |_| {}).map_err(|e| e.to_string())?;
    if status != 200 {
        return Err(format!("HTTP {status}"));
    }
    if !d.matches(&bytes) {
        return Err(format!("it came back different ({} bytes, expected {})", bytes.len(), d.size));
    }
    let dir = path.parent().ok_or("no folder")?;
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let part = path.with_extension("part");
    fs::write(&part, &bytes).and_then(|()| fs::rename(&part, path)).map_err(|e| {
        let _ = fs::remove_file(&part);
        e.to_string()
    })
}

/// Run the Setup at `setup` silently, once it still matches `d`. Like any
/// install it closes this copy (it saves first); it keeps Start with
/// Windows as it is and opens the new version when done (`/reopen`,
/// installer/lumo.nsi).
#[cfg(windows)]
pub fn install(setup: &std::path::Path, d: &update::Download) -> Result<(), String> {
    if !update::ENABLED { return Err("Updates are temporarily disabled".into()); }
    if !d.allowed() || setup.file_name().is_none_or(|name| name != d.file_name()) { return Err("Untrusted update installer".into()); }
    if !std::fs::read(setup).is_ok_and(|b| d.matches(&b)) {
        return Err("the downloaded Setup is gone or changed".into());
    }
    std::process::Command::new(setup).args(["/S", "/reopen"]).spawn().map(drop).map_err(|e| e.to_string())
}

/// ECDSA P-256 over SHA-256: `signature` is r and s, `key` an uncompressed point.
#[cfg(all(windows, test))]
fn verify(message: &[u8], signature: &[u8; 64], key: &[u8; 65]) -> bool {
    use windows::Win32::Security::Cryptography::{
        BCryptCloseAlgorithmProvider, BCryptDestroyKey, BCryptImportKeyPair, BCryptOpenAlgorithmProvider, BCryptVerifySignature, BCRYPT_ALG_HANDLE,
        BCRYPT_ECCPUBLIC_BLOB, BCRYPT_ECDSA_P256_ALGORITHM, BCRYPT_ECDSA_PUBLIC_P256_MAGIC, BCRYPT_FLAGS, BCRYPT_KEY_HANDLE,
        BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS,
    };
    // BCRYPT_ECCKEY_BLOB: magic, key length, then X and Y.
    let mut blob = Vec::with_capacity(72);
    blob.extend_from_slice(&BCRYPT_ECDSA_PUBLIC_P256_MAGIC.to_le_bytes());
    blob.extend_from_slice(&32u32.to_le_bytes());
    blob.extend_from_slice(&key[1..]);
    let hash = lumo_core::sha256::sha256(message);
    unsafe {
        let mut alg = BCRYPT_ALG_HANDLE::default();
        if BCryptOpenAlgorithmProvider(&mut alg, BCRYPT_ECDSA_P256_ALGORITHM, None, BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0)).is_err() {
            return false;
        }
        let mut handle = BCRYPT_KEY_HANDLE::default();
        let ok = BCryptImportKeyPair(alg, BCRYPT_KEY_HANDLE::default(), BCRYPT_ECCPUBLIC_BLOB, &mut handle, &blob, 0).is_ok()
            && BCryptVerifySignature(handle, None, &hash, signature, BCRYPT_FLAGS(0)).is_ok();
        if !handle.is_invalid() {
            let _ = BCryptDestroyKey(handle);
        }
        let _ = BCryptCloseAlgorithmProvider(alg, 0);
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "contacts the public GitHub releases API"]
    fn checks_lumo_releases_live() {
        let _com = crate::services::http::MtaGuard::new();
        let client = Client::new(concat!("Lumo/", env!("CARGO_PKG_VERSION"))).unwrap();
        let release = check(&client).expect("Lumo's public GitHub release list");
        if let Some(release) = release {
            assert!(release.allowed());
            println!("Latest verified installer: {}", release.version);
        } else { println!("No stable release with an uploaded, hashed installer yet"); }
    }

    #[test]
    fn lumo_signature_verifies_and_rejects_changes() {
        let message = b"Lumo update signature verification fixture";
        let key = [0x04, 0xb2, 0xd5, 0x35, 0x64, 0xc5, 0x7c, 0xbe, 0xa8, 0xf7, 0xeb, 0xe4, 0xf7, 0x4c, 0x01, 0x00, 0xe8, 0x59, 0x7f, 0x06, 0x61, 0x6d, 0xbd, 0xca, 0x4d, 0x52, 0xd0, 0xe7, 0xd4, 0x8d, 0x83, 0x0b, 0x6e, 0x04, 0x7f, 0x3d, 0x5d, 0xf0, 0xce, 0x71, 0x33, 0x0d, 0x7c, 0xb1, 0x17, 0xe7, 0xf4, 0xa0, 0xd1, 0x79, 0x1d, 0x6b, 0xff, 0x49, 0x2d, 0xd6, 0xed, 0x21, 0xfd, 0x26, 0x0f, 0x3a, 0xd1, 0x99, 0x16];
        let signature = [0xfc, 0x6b, 0x03, 0x1d, 0x05, 0xd9, 0xa0, 0x95, 0x12, 0x22, 0x76, 0xd9, 0x19, 0x99, 0x28, 0x1b, 0x3f, 0xc9, 0x39, 0x24, 0x0e, 0x75, 0xc4, 0x86, 0x1e, 0xe1, 0x7d, 0x50, 0x11, 0x02, 0x60, 0x42, 0xb1, 0x41, 0xeb, 0x0a, 0xdc, 0x29, 0x83, 0xe2, 0x41, 0xfe, 0x6f, 0x12, 0x81, 0xc1, 0x4e, 0x23, 0xed, 0x9c, 0xa7, 0x6b, 0x5a, 0x98, 0x27, 0x47, 0x7e, 0x04, 0xcc, 0xd1, 0x70, 0xfd, 0xfd, 0x2f];
        assert!(verify(message, &signature, &key));
        assert!(!verify(b"Changed Lumo update", &signature, &key));
        let mut changed = signature;
        changed[10] ^= 1;
        assert!(!verify(message, &changed, &key));
        let mut other_key = key;
        other_key[10] ^= 1;
        assert!(!verify(message, &signature, &other_key));
    }
}
