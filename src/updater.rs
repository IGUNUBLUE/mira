//! Optional, signed updates. Network and filesystem work stays off the UI thread.
mod install;
mod io;

pub use install::{Installed, Prepared, Target, acknowledge, has_health, register_health, valid_health_token};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const REPO: &str = "https://github.com/IGUNUBLUE/mira";
const API: &str = "https://api.github.com/repos/IGUNUBLUE/mira/releases/latest";
pub const PUBLIC_KEY: &str = include_str!("../assets/update-public.pem");
pub const INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_META: usize = 4096;
const MAX_ARCHIVE: u64 = 64 * 1024 * 1024;
const MAX_BINARY: u64 = 128 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Version(u64, u64, u64);
impl Version {
    fn parse(value: &str) -> Option<Self> {
        let parts: Vec<_> = value.split('.').collect();
        if parts.len() != 3 {
            return None;
        }
        let number = |s: &str| {
            if s.is_empty() || (s.len() > 1 && s.starts_with('0')) || !s.bytes().all(|b| b.is_ascii_digit()) {
                None
            } else {
                s.parse().ok()
            }
        };
        Some(Self(number(parts[0])?, number(parts[1])?, number(parts[2])?))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u8,
    pub version: String,
    pub arch: String,
    pub archive: String,
    pub archive_sha256: String,
    pub binary_sha256: String,
    pub archive_size: u64,
    pub binary_size: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Signed {
    pub json: String,
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct Release {
    pub manifest: Manifest,
    pub signed: Signed,
    pub label: String,
    pub notes: String,
}
impl Release {
    pub fn newer_than(&self, current: &str) -> bool {
        match (Version::parse(&self.manifest.version), Version::parse(current)) {
            (Some(next), Some(current)) => next > current,
            _ => false,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Cache {
    pub checked_at: u64,
    pub successful_at: u64,
    pub retry_after: u64,
    pub etag: Option<String>,
    pub latest: Option<Signed>,
    pub pending: Option<Signed>,
}

#[derive(Debug)]
pub struct Loaded {
    pub cache: Cache,
    pub available: Option<Release>,
    pub target: Result<Target, String>,
    pub ready: Option<Prepared>,
}

pub fn automatic() -> bool {
    crate::persist::config_get("update_checks").as_deref() == Some("on")
}

pub fn due(checked: u64, now: u64) -> bool {
    checked == 0 || now < checked || now.saturating_sub(checked) >= INTERVAL.as_secs()
}

pub fn state_dir() -> Result<PathBuf, String> {
    crate::persist::state_dir().map(|p| p.join("updates")).ok_or_else(|| "No state directory for updates".into())
}

pub fn load() -> Result<Loaded, String> {
    let dir = state_dir()?;
    io::private_dir(&dir)?;
    let mut cache: Cache = match io::read_bounded(&dir.join("state.json"), 32 * 1024) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("Update cache: {e}"))?,
        Err(_) if !dir.join("state.json").exists() => Cache::default(),
        Err(e) => return Err(e),
    };
    let cancel = AtomicBool::new(false);
    let verified_latest = cache.latest.as_ref().and_then(|s| verify(s, PUBLIC_KEY, &cancel).ok());
    if cache.latest.is_some() && verified_latest.is_none() {
        cache.latest = None;
        cache.etag = None;
        cache.checked_at = 0;
    }
    let available = verified_latest.filter(|r| r.newer_than(env!("CARGO_PKG_VERSION")));
    let pending = cache.pending.as_ref().and_then(|s| verify(s, PUBLIC_KEY, &cancel).ok());
    let target = std::env::current_exe().map_err(|e| e.to_string()).and_then(|p| {
        Target::detect(&p).or_else(|original| {
            pending
                .as_ref()
                .filter(|r| r.manifest.version == env!("CARGO_PKG_VERSION"))
                .and_then(|r| install::repair_record(&p, r, &cancel).ok())
                .ok_or(original)
        })
    });
    let ready = match (&target, pending) {
        (Ok(target), Some(release)) if release.newer_than(env!("CARGO_PKG_VERSION")) => {
            install::restore(target, release, &cancel).ok()
        }
        _ => {
            cache.pending = None;
            None
        }
    };
    Ok(Loaded { cache, available, target, ready })
}

pub fn save(cache: &Cache) -> Result<(), String> {
    let bytes = serde_json::to_vec(cache).map_err(|e| e.to_string())?;
    io::atomic_write(&state_dir()?.join("state.json"), &bytes, 0o600)
}

fn name(version: &str, arch: &str) -> String {
    format!("mira-v{version}-{arch}-linux")
}
fn asset_url(version: &str, file: &str) -> String {
    format!("{REPO}/releases/download/v{version}/{file}")
}
fn valid_hash(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn verify(signed: &Signed, key: &str, cancel: &AtomicBool) -> Result<Release, String> {
    if signed.json.len() > MAX_META || signed.signature.len() != 64 {
        return Err("Invalid update metadata size".into());
    }
    io::verify_signature(signed.json.as_bytes(), &signed.signature, key, cancel)?;
    let manifest: Manifest = serde_json::from_str(&signed.json).map_err(|e| format!("Update metadata: {e}"))?;
    if manifest.format != 1
        || Version::parse(&manifest.version).is_none()
        || !["x86_64", "aarch64"].contains(&manifest.arch.as_str())
        || manifest.arch != std::env::consts::ARCH
        || manifest.archive != format!("{}.tar.gz", name(&manifest.version, &manifest.arch))
        || !valid_hash(&manifest.archive_sha256)
        || !valid_hash(&manifest.binary_sha256)
        || !(1..=MAX_ARCHIVE).contains(&manifest.archive_size)
        || !(64..=MAX_BINARY).contains(&manifest.binary_size)
    {
        return Err("Update metadata does not match this platform".into());
    }
    let label = format!("Mira {} is available", manifest.version);
    let notes = format!("{REPO}/releases/tag/v{}", manifest.version);
    Ok(Release { manifest, signed: signed.clone(), label, notes })
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<ApiAsset>,
}
#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
}

fn select(bytes: &[u8], current: &str) -> Result<Option<(String, String)>, String> {
    let api: ApiRelease = serde_json::from_slice(bytes).map_err(|e| format!("Release response: {e}"))?;
    if api.draft || api.prerelease {
        return Ok(None);
    }
    let version = api.tag_name.strip_prefix('v').and_then(Version::parse).ok_or("Invalid release version")?;
    let current = Version::parse(current).ok_or("Invalid installed version")?;
    if version <= current {
        return Ok(None);
    }
    let version = api.tag_name.strip_prefix('v').ok_or("Invalid release tag")?.to_string();
    if api.assets.len() > 32 {
        return Err("Too many release assets".into());
    }
    let base = name(&version, std::env::consts::ARCH);
    for suffix in [".tar.gz", ".update.json", ".update.json.sig"] {
        let file = format!("{base}{suffix}");
        if !api.assets.iter().any(|a| a.name == file && a.browser_download_url == asset_url(&version, &file)) {
            return Err("This release has no signed update package for this architecture".into());
        }
    }
    Ok(Some((version, base)))
}

pub fn check(mut cache: Cache, cancel: &AtomicBool) -> Result<Cache, String> {
    let now = crate::persist::now();
    // A failed check may have persisted a rate limit, or another window may have observed it.
    if let Ok(bytes) = io::read_bounded(&state_dir()?.join("state.json"), 32 * 1024)
        && let Ok(previous) = serde_json::from_slice::<Cache>(&bytes)
    {
        cache.retry_after = cache.retry_after.max(previous.retry_after);
    }
    if now < cache.retry_after {
        return Err("GitHub requested a cooldown; try again later".into());
    }
    cache.checked_at = now;
    // Persist attempts too: offline startups must not cause a request on every focus event.
    save(&cache)?;
    let response = io::get(API, cache.etag.as_deref(), 1024 * 1024, cancel)?;
    if response.status == 304 {
        cache.successful_at = now;
        save(&cache)?;
        return Ok(cache);
    }
    if response.status == 403 || response.status == 429 {
        cache.retry_after = response.retry_after.max(now.saturating_add(60));
        save(&cache)?;
        return Err("GitHub rate limit reached; try again after its cooldown".into());
    }
    if response.status != 200 {
        return Err(format!("Update server returned HTTP {}", response.status));
    }
    let next = match select(&response.body, env!("CARGO_PKG_VERSION"))? {
        None => None,
        Some((version, base)) => {
            let json = io::get(&asset_url(&version, &format!("{base}.update.json")), None, MAX_META, cancel)?;
            let sig = io::get(&asset_url(&version, &format!("{base}.update.json.sig")), None, 64, cancel)?;
            if json.status != 200 || sig.status != 200 {
                return Err("Signed update metadata is unavailable".into());
            }
            let signed = Signed {
                json: String::from_utf8(json.body).map_err(|_| "Invalid update metadata encoding")?,
                signature: sig.body,
            };
            let release = verify(&signed, PUBLIC_KEY, cancel)?;
            if release.manifest.version != version {
                return Err("Signed version does not match the release".into());
            }
            Some(signed)
        }
    };
    cache.successful_at = now;
    cache.etag = response.etag;
    cache.latest = next;
    cache.retry_after = 0;
    save(&cache)?;
    Ok(cache)
}

pub fn available(cache: &Cache) -> Result<Option<Release>, String> {
    cache
        .latest
        .as_ref()
        .map(|s| verify(s, PUBLIC_KEY, &AtomicBool::new(false)))
        .transpose()
        .map(|r| r.filter(|r| r.newer_than(env!("CARGO_PKG_VERSION"))))
}

pub fn download(
    release: Release,
    target: Target,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<Prepared, String> {
    let release = verify(&release.signed, PUBLIC_KEY, cancel)?;
    if !release.newer_than(env!("CARGO_PKG_VERSION")) {
        return Err("The update is not newer than this Mira".into());
    }
    target.revalidate(cancel)?;
    let work = io::Workspace::new(&state_dir()?)?;
    let archive = work.path.join("package.tar.gz");
    let mut file = io::new_file(&archive, 0o600)?;
    let limit = release.manifest.archive_size;
    let mut written = 0_u64;
    let mut error = None;
    let mut last_progress = std::time::Instant::now();
    let mut command = io::curl(&asset_url(&release.manifest.version, &release.manifest.archive));
    command.arg("--fail");
    let result = io::stream(command, cancel, Duration::from_secs(600), &mut |bytes| {
        use std::io::Write as _;
        if error.is_some() {
            return;
        }
        written = written.saturating_add(bytes.len() as u64);
        if written > limit {
            error = Some("Update exceeded its signed size".to_string());
        } else if let Err(e) = file.write_all(bytes) {
            error = Some(e.to_string());
        }
        if error.is_some() {
            cancel.store(true, Ordering::Relaxed);
        }
        if last_progress.elapsed() >= Duration::from_millis(100) {
            progress(written.min(limit), limit);
            last_progress = std::time::Instant::now();
        }
    });
    if let Some(error) = error {
        return Err(error);
    }
    result?;
    file.sync_all().map_err(|e| e.to_string())?;
    if written != limit || io::sha256(&archive, cancel)? != release.manifest.archive_sha256 {
        return Err("Update package checksum or size mismatch".into());
    }
    progress(limit, limit);
    install::prepare(target, release, &archive, cancel)
}

pub fn install(prepared: &Prepared, cancel: &AtomicBool) -> Result<Installed, String> {
    let verified = verify(&prepared.release.signed, PUBLIC_KEY, cancel)?;
    if !verified.newer_than(env!("CARGO_PKG_VERSION")) {
        return Err("Refusing to install an older version".into());
    }
    let authenticated = Prepared { release: verified, ..prepared.clone() };
    install::apply(&authenticated, cancel)
}

pub fn cache_ready(mut cache: Cache, prepared: &Prepared) -> Result<Cache, String> {
    cache.pending = Some(prepared.release.signed.clone());
    save(&cache)?;
    Ok(cache)
}

pub fn restart(installed: &Installed, root: &Path, follow: bool) -> Result<(), String> {
    install::restart(installed, root, follow)
}

#[cfg(test)]
#[path = "updater/tests.rs"]
mod tests;
