use super::*;
use std::fs;
use std::process::Command;
use std::sync::OnceLock;

fn keys() -> &'static (String, String) {
    static KEYS: OnceLock<(String, String)> = OnceLock::new();
    KEYS.get_or_init(|| {
        let dir = crate::files::test_dir("update-key");
        let private = dir.join("private.pem");
        assert!(
            Command::new("openssl")
                .args(["genpkey", "-algorithm", "ED25519", "-out"])
                .arg(&private)
                .status()
                .unwrap()
                .success()
        );
        let public = Command::new("openssl").args(["pkey", "-in"]).arg(&private).arg("-pubout").output().unwrap();
        assert!(public.status.success());
        (private.to_string_lossy().into_owned(), String::from_utf8(public.stdout).unwrap())
    })
}
fn manifest() -> Manifest {
    Manifest {
        format: 1,
        version: "99.0.0".into(),
        arch: std::env::consts::ARCH.into(),
        archive: format!("{}.tar.gz", name("99.0.0", std::env::consts::ARCH)),
        archive_sha256: "a".repeat(64),
        binary_sha256: "b".repeat(64),
        archive_size: 100,
        binary_size: 128,
    }
}
fn signed(json: String) -> Signed {
    let dir = crate::files::test_dir("update-sign");
    fs::write(dir.join("input"), &json).unwrap();
    assert!(
        Command::new("openssl")
            .args(["pkeyutl", "-sign", "-rawin", "-inkey", &keys().0, "-in"])
            .arg(dir.join("input"))
            .arg("-out")
            .arg(dir.join("sig"))
            .status()
            .unwrap()
            .success()
    );
    let signature = fs::read(dir.join("sig")).unwrap();
    fs::remove_dir_all(dir).unwrap();
    Signed { json, signature }
}
#[test]
fn authenticates_metadata_and_rejects_modified_bytes_wrong_key_or_signature() {
    let cancel = AtomicBool::new(false);
    let valid = signed(serde_json::to_string(&manifest()).unwrap());
    let release = verify(&valid, &keys().1, &cancel).unwrap();
    assert!(release.newer_than("0.99.0"));
    assert!(!release.newer_than("99.0.0"));
    assert!(!release.newer_than("100.0.0"));
    assert!(verify(&valid, PUBLIC_KEY, &cancel).is_err());
    let mut changed = valid.clone();
    changed.json = changed.json.replace("99.0.0", "99.0.1");
    assert!(verify(&changed, &keys().1, &cancel).is_err());
    let mut changed = valid.clone();
    changed.signature[0] ^= 1;
    assert!(verify(&changed, &keys().1, &cancel).is_err());
    let mut changed = valid;
    changed.signature.pop();
    assert!(verify(&changed, &keys().1, &cancel).is_err());
}
#[test]
fn signed_metadata_still_requires_supported_platform_names_and_bounded_sizes() {
    let cancel = AtomicBool::new(false);
    let mut variants = Vec::new();
    let mut m = manifest();
    m.arch = "unknown".into();
    variants.push(m);
    let mut m = manifest();
    m.archive = "../../mira".into();
    variants.push(m);
    let mut m = manifest();
    m.archive_size = MAX_ARCHIVE + 1;
    variants.push(m);
    let mut m = manifest();
    m.binary_size = 0;
    variants.push(m);
    let mut m = manifest();
    m.binary_sha256 = "B".repeat(64);
    variants.push(m);
    let mut m = manifest();
    m.format = 2;
    variants.push(m);
    let mut m = manifest();
    m.version = "99.0.0-beta".into();
    variants.push(m);
    for m in variants {
        assert!(verify(&signed(serde_json::to_string(&m).unwrap()), &keys().1, &cancel).is_err());
    }
    let json = serde_json::to_string(&manifest()).unwrap().replace("\"format\":1", "\"extra\":true,\"format\":1");
    assert!(verify(&signed(json), &keys().1, &cancel).is_err());
}
#[test]
fn stable_versions_order_numerically_and_reject_ambiguous_tags() {
    assert!(Version::parse("0.11.0") > Version::parse("0.9.99"));
    for bad in ["", "v1.2.3", "1.2", "01.2.3", "1.2.3.4", "1.2.3-beta", "-1.2.3", "1.2.3/path"] {
        assert!(Version::parse(bad).is_none(), "{bad}");
    }
}
fn api(version: &str) -> serde_json::Value {
    let base = name(version, std::env::consts::ARCH);
    let assets = [".tar.gz", ".update.json", ".update.json.sig"].map(|suffix| {
        let file = format!("{base}{suffix}");
        serde_json::json!({"name":file,"browser_download_url":asset_url(version,&file)})
    });
    serde_json::json!({"tag_name":format!("v{version}"),"draft":false,"prerelease":false,"assets":assets})
}
#[test]
fn release_selection_requires_complete_canonical_assets_and_ignores_old_or_preview_releases() {
    let mut value = api("99.0.0");
    assert!(select(&serde_json::to_vec(&value).unwrap(), "1.0.0").unwrap().is_some());
    assert!(select(&serde_json::to_vec(&value).unwrap(), "99.0.0").unwrap().is_none());
    value["prerelease"] = true.into();
    assert!(select(&serde_json::to_vec(&value).unwrap(), "1.0.0").unwrap().is_none());
    value["prerelease"] = false.into();
    value["assets"][0]["browser_download_url"] = "https://evil.invalid/mira".into();
    assert!(select(&serde_json::to_vec(&value).unwrap(), "1.0.0").is_err());
    value["assets"] = serde_json::json!([]);
    assert!(select(&serde_json::to_vec(&value).unwrap(), "1.0.0").is_err());
}
#[test]
fn request_schedule_headers_cancellation_and_file_limits() {
    assert!(due(0, 2));
    assert!(!due(100, 101));
    assert!(due(100, 100 + INTERVAL.as_secs()));
    assert!(due(100, 99));
    let (status, etag, retry) =
        io::parse_headers("HTTP/1.1 302 Found\r\nETag: old\r\n\r\nHTTP/2 429\r\nETag: new\r\nRetry-After: 120\r\n");
    assert_eq!(status, 429);
    assert_eq!(etag.as_deref(), Some("new"));
    assert!(retry >= crate::persist::now() + 119);
    assert_eq!(io::parse_headers("HTTP/2 304\r\n").0, 304);
    let cancel = AtomicBool::new(true);
    assert!(
        io::output(Command::new("this-command-must-never-start"), 1, &cancel)
            .unwrap_err()
            .contains(crate::proc::CANCELLED)
    );
    let dir = crate::files::test_dir("update-files");
    fs::write(dir.join("large"), b"12345").unwrap();
    assert!(io::read_bounded(&dir.join("large"), 4).is_err());
    std::os::unix::fs::symlink(dir.join("large"), dir.join("link")).unwrap();
    assert!(io::read_bounded(&dir.join("link"), 9).is_err());
    assert!(io::private_dir(&dir.join("link")).is_err());
    fs::remove_dir_all(dir).unwrap();
}
