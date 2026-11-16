//! Prebuilt web frontend download from GitHub releases (ARC-005 split).
//!
//! Powers `par-term-streamer --download-frontend`: fetches the frontend
//! archive attached to a release of this repository and extracts it into the
//! configured web root.

use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use std::fs;
use std::path::Path;
use std::time::Duration;
use tar::Archive;

/// GitHub API response for release information
#[derive(serde::Deserialize, Debug)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

/// GitHub API response for release asset
#[derive(serde::Deserialize, Debug)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

const GITHUB_REPO: &str = "paulrobello/par-term-emu-core-rust";
const FRONTEND_ARCHIVE_PREFIX: &str = "par-term-web-frontend-v";

/// Hard cap on the downloaded archive (SEC-007). The real bundle is a few
/// MiB; anything larger is a misdirected URL or a malicious response and
/// must not be buffered in memory, let alone extracted.
/// cap: Bytes accepted from a downloaded web-frontend archive response.
const MAX_ARCHIVE_BYTES: usize = 50 * 1024 * 1024;
/// Total bytes the extracted bundle may occupy: the 50 MiB download cap
/// bounds compressed bytes only, so a hostile archive could still expand
/// past it (text assets compress far better than 4x).
const MAX_EXTRACTED_BYTES: u64 = 256 * 1024 * 1024;

/// Read a response body up to `max_bytes`, failing as soon as the cap is
/// exceeded instead of buffering an unbounded stream (SEC-007).
async fn read_body_capped(response: reqwest::Response, max_bytes: usize) -> Result<Vec<u8>> {
    if let Some(len) = response.content_length() {
        if len as usize > max_bytes {
            anyhow::bail!(
                "Frontend archive is {} bytes, over the {} MiB download cap",
                len,
                max_bytes / (1024 * 1024)
            );
        }
    }
    let mut response = response;
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("Failed to read archive content")?
    {
        if body.len() + chunk.len() > max_bytes {
            anyhow::bail!(
                "Frontend archive exceeded the {} MiB download cap mid-stream",
                max_bytes / (1024 * 1024)
            );
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// cap: Bytes accepted from a `.sha256` checksum sidecar response (SEC-203).
const MAX_SIDECAR_BYTES: usize = 1024;

/// Parse a `sha256sum`-style sidecar (`<64 hex>  <filename>`) and return the
/// lowercase digest. A malformed sidecar is an error, never a skip (SEC-203):
/// falling back to "unverified" on garbage would let an attacker downgrade
/// verification by corrupting the sidecar.
fn parse_sha256_sidecar(body: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(body).context("Checksum sidecar is not valid UTF-8")?;
    let digest = text
        .split_whitespace()
        .next()
        .ok_or_else(|| anyhow::anyhow!("Checksum sidecar is empty"))?;
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        anyhow::bail!("Checksum sidecar does not start with a 64-hex-digit SHA-256 digest");
    }
    Ok(digest.to_ascii_lowercase())
}

/// First release whose frontend archive ships a `.sha256` sidecar (v0.58.1;
/// v0.58.0 and earlier have none). From this version on, a missing sidecar is
/// a stripped or tampered release, not an old one (SEC-004).
const SIDECAR_REQUIRED_SINCE: (u64, u64, u64) = (0, 58, 1);

/// Parse a release tag (`v1.2.3` or `1.2.3`) into its numeric triple. Any
/// pre-release/build suffix or non-numeric part yields `None`.
fn parse_release_version(tag: &str) -> Option<(u64, u64, u64)> {
    let tag = tag.strip_prefix('v').unwrap_or(tag);
    let mut parts = tag.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Decide what a release without a checksum sidecar means (SEC-004). Releases
/// older than [`SIDECAR_REQUIRED_SINCE`] predate the sidecar and fall back to
/// TLS-only integrity with a returned warning; every other release, including
/// an unparseable or pre-release tag, fails closed.
fn missing_sidecar_outcome(tag: &str, sidecar_name: &str) -> Result<String> {
    match parse_release_version(tag) {
        Some(version) if version < SIDECAR_REQUIRED_SINCE => Ok(format!(
            "Warning: release {tag} predates checksum sidecars and has no {sidecar_name}; \
             the archive integrity is protected by TLS only."
        )),
        _ => anyhow::bail!(
            "Release {tag} has no {sidecar_name} checksum asset, which every release since \
             v{}.{}.{} publishes. Refusing to extract an unverified frontend archive.",
            SIDECAR_REQUIRED_SINCE.0,
            SIDECAR_REQUIRED_SINCE.1,
            SIDECAR_REQUIRED_SINCE.2
        ),
    }
}

/// Fail unless `bytes` hashes to `expected_hex` (lowercase SHA-256 hex).
fn verify_archive_sha256(bytes: &[u8], expected_hex: &str) -> Result<()> {
    use sha2::Digest;
    let actual: String = sha2::Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if actual != expected_hex {
        anyhow::bail!(
            "Frontend archive SHA-256 mismatch: expected {}, got {}. Refusing to extract.",
            expected_hex,
            actual
        );
    }
    Ok(())
}

/// Download a checksum sidecar and return its digest. Any failure (HTTP
/// error, oversized or malformed body) is fatal (SEC-203).
async fn fetch_expected_sha256(client: &reqwest::Client, url: &str) -> Result<String> {
    let response = client
        .get(url)
        .send()
        .await
        .context("Failed to download checksum sidecar")?;
    if !response.status().is_success() {
        anyhow::bail!(
            "Failed to download checksum sidecar: HTTP {}",
            response.status()
        );
    }
    let body = read_body_capped(response, MAX_SIDECAR_BYTES)
        .await
        .context("Checksum sidecar response exceeded its 1 KiB cap")?;
    parse_sha256_sidecar(&body)
}

/// Guard the destructive web-root replacement (SEC-007): an existing
/// directory is only cleared when it looks like a previously extracted
/// frontend bundle (has `index.html`) or `force` was passed explicitly.
/// A stray `--web-root` pointing at an unrelated directory is refused.
fn check_web_root_replaceable(web_root: &Path, force: bool) -> Result<()> {
    if web_root.exists() && !force && !web_root.join("index.html").exists() {
        anyhow::bail!(
            "Refusing to replace web root '{}': it exists but has no index.html, so it does not \
             look like a previously extracted frontend bundle. Re-run with --force-web-download \
             to overwrite it anyway.",
            web_root.display()
        );
    }
    Ok(())
}

/// Download and extract the web frontend from GitHub releases
pub async fn download_frontend(version: &str, web_root: &str, force: bool) -> Result<()> {
    let client = reqwest::Client::builder()
        .user_agent("par-term-streamer")
        .timeout(Duration::from_secs(60))
        .build()
        .context("Failed to create HTTP client")?;

    // Get release info from GitHub API
    let release_url = if version == "latest" {
        format!(
            "https://api.github.com/repos/{}/releases/latest",
            GITHUB_REPO
        )
    } else {
        format!(
            "https://api.github.com/repos/{}/releases/tags/v{}",
            GITHUB_REPO, version
        )
    };

    println!("Fetching release info from GitHub...");
    let response = client
        .get(&release_url)
        .send()
        .await
        .context("Failed to fetch release info from GitHub")?;

    if !response.status().is_success() {
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            if version == "latest" {
                anyhow::bail!("No releases found for this repository");
            } else {
                anyhow::bail!("Release version '{}' not found", version);
            }
        }
        anyhow::bail!(
            "GitHub API request failed with status: {}",
            response.status()
        );
    }

    let release: GitHubRelease = response
        .json()
        .await
        .context("Failed to parse GitHub release info")?;

    println!("Found release: {}", release.tag_name);

    // Find the tar.gz frontend archive
    let archive_asset = release
        .assets
        .iter()
        .find(|asset| {
            asset.name.starts_with(FRONTEND_ARCHIVE_PREFIX) && asset.name.ends_with(".tar.gz")
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Web frontend archive not found in release {}. Available assets: {}",
                release.tag_name,
                release
                    .assets
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        })?;

    println!("Downloading: {}", archive_asset.name);
    println!("From: {}", archive_asset.browser_download_url);

    // Download the archive
    let response = client
        .get(&archive_asset.browser_download_url)
        .send()
        .await
        .context("Failed to download frontend archive")?;

    if !response.status().is_success() {
        anyhow::bail!("Failed to download archive: HTTP {}", response.status());
    }

    let content_length = response.content_length();
    if let Some(len) = content_length {
        println!("Download size: {} bytes", len);
    }

    // SEC-007: stream the body with a hard size cap instead of buffering
    // whatever the URL serves.
    let archive_bytes = read_body_capped(response, MAX_ARCHIVE_BYTES).await?;

    println!("Downloaded {} bytes", archive_bytes.len());

    // SEC-203 / SEC-004: verify against the release's `<archive>.sha256`
    // sidecar before touching the web root. A listed-but-unusable sidecar is
    // fatal, and so is a missing one on any release since the sidecar shipped;
    // only releases predating it fall back to TLS-only with a warning.
    let sidecar_name = format!("{}.sha256", archive_asset.name);
    match release.assets.iter().find(|a| a.name == sidecar_name) {
        Some(sidecar) => {
            let expected = fetch_expected_sha256(&client, &sidecar.browser_download_url).await?;
            verify_archive_sha256(&archive_bytes, &expected)?;
            println!("SHA-256 verified: {}", expected);
        }
        None => {
            eprintln!(
                "{}",
                missing_sidecar_outcome(&release.tag_name, &sidecar_name)?
            );
        }
    }

    // Create web root directory if it doesn't exist
    let web_root_path = Path::new(web_root);
    // SEC-007: refuse to clear a directory that does not look like a
    // previously extracted frontend unless --force-web-download was passed.
    check_web_root_replaceable(web_root_path, force)?;
    if web_root_path.exists() {
        println!("Clearing existing web root: {}", web_root);
        fs::remove_dir_all(web_root_path)
            .context(format!("Failed to remove existing directory: {}", web_root))?;
    }
    fs::create_dir_all(web_root_path)
        .context(format!("Failed to create web root directory: {}", web_root))?;

    // Extract the tar.gz archive
    println!("Extracting to: {}", web_root);
    let tar_gz = GzDecoder::new(archive_bytes.as_slice());
    let mut archive = Archive::new(tar_gz);

    // SEC-006: the `.sha256` sidecar comes from the same release as the
    // archive, so it cannot vouch against a compromised release — `unpack`
    // would honor symlink/hardlink entries, writing outside the web root.
    extract_regular_files(&mut archive, web_root_path, MAX_EXTRACTED_BYTES)?;

    // Count extracted files
    let file_count = count_files(web_root_path)?;
    println!(
        "Successfully extracted {} files to {}",
        file_count, web_root
    );

    // Verify index.html exists
    let index_path = web_root_path.join("index.html");
    if !index_path.exists() {
        println!("Warning: index.html not found in extracted content");
    } else {
        println!("Frontend ready at: {}/index.html", web_root);
    }

    Ok(())
}

/// A tar entry path that stays inside the extraction root: relative, with
/// every component a normal name (rejects `..`, `.`, absolute paths, and
/// Windows prefixes). Rejects rather than skips, so a tampered archive is
/// loud instead of silently partial.
fn safe_entry_path(path: &Path) -> Result<&Path> {
    let normal = path
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)));
    if normal && !path.as_os_str().is_empty() {
        Ok(path)
    } else {
        anyhow::bail!(
            "Refusing to extract archive entry '{}': the path escapes the extraction root",
            path.display()
        );
    }
}

/// Extract `archive` into `dest`, writing regular files and directories
/// only (SEC-006). Link, device, and FIFO entries abort the extraction:
/// `Archive::unpack` would honor a symlink/hardlink entry and write outside
/// `dest` (or replace paths the release never shipped), and the checksum
/// sidecar comes from the same release as the archive, so it cannot vouch
/// against a compromised one. The bundle our release process publishes
/// contains only regular files and directories, so a legitimate archive
/// never trips this.
fn extract_regular_files<R: std::io::Read>(
    archive: &mut Archive<R>,
    dest: &Path,
    max_total_bytes: u64,
) -> Result<()> {
    use std::io::Read as _;
    let mut extracted_total: u64 = 0;
    for entry in archive
        .entries()
        .context("Failed to read archive entries")?
    {
        let mut entry = entry.context("Failed to read archive entry")?;
        let entry_type = entry.header().entry_type();
        let path = entry
            .path()
            .context("Failed to read archive entry path")?
            .into_owned();
        if entry_type.is_file() {
            let out_path = dest.join(safe_entry_path(&path)?);
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "Over-extraction cap aborts before '{}' is created",
                        path.display()
                    )
                })?;
            }
            let mut out = fs::File::create(&out_path)
                .with_context(|| format!("Failed to create {}", out_path.display()))?;
            // `take` bounds a single entry whose header lies about its size:
            // at most remaining+1 bytes are read, so the total can overshoot
            // the cap by at most one byte before the check below aborts.
            let remaining = max_total_bytes.saturating_sub(extracted_total);
            let copied = {
                let mut limited = (&mut entry).take(remaining.saturating_add(1));
                std::io::copy(&mut limited, &mut out)
                    .with_context(|| format!("Failed to write {}", out_path.display()))?
            };
            extracted_total += copied;
            if extracted_total > max_total_bytes {
                anyhow::bail!(
                    "Refusing to extract archive entry '{}': the bundle expands past the \
                     {} MiB extraction cap (CWE-409)",
                    path.display(),
                    max_total_bytes / (1024 * 1024)
                );
            }
            // Match `Archive::unpack`'s default: apply the header's plain
            // permission bits (never setuid/setgid/sticky).
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = entry.header().mode().unwrap_or(0o644) & 0o777;
                out.set_permissions(fs::Permissions::from_mode(mode))
                    .with_context(|| {
                        format!("Failed to set permissions on {}", out_path.display())
                    })?;
            }
        } else if entry_type.is_dir() {
            fs::create_dir_all(dest.join(safe_entry_path(&path)?))
                .with_context(|| format!("Failed to create directory {}", path.display()))?;
        } else {
            anyhow::bail!(
                "Refusing to extract archive entry '{}' with type byte {:#x}: only regular \
                 files and directories are extracted; the frontend bundle never ships links, \
                 devices, or FIFOs",
                path.display(),
                entry_type.as_byte()
            );
        }
    }
    Ok(())
}

/// Count files recursively in a directory
fn count_files(path: &Path) -> Result<usize> {
    let mut count = 0;
    if path.is_dir() {
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                count += count_files(&path)?;
            } else {
                count += 1;
            }
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal single-shot HTTP server: accepts one connection, serves
    /// `body` with the given Content-Length, closes.
    fn spawn_stub_server(body: Vec<u8>) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut conn, _)) = listener.accept() {
                let mut buf = [0u8; 1024];
                // Drain the request head (best-effort).
                let _ = conn.read(&mut buf);
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = conn.write_all(head.as_bytes());
                let _ = conn.write_all(&body);
            }
        });
        format!("http://{}", addr)
    }

    #[tokio::test]
    async fn oversized_body_is_rejected_before_extraction() {
        // Serve 4 KiB with a 1 KiB cap — the same failure mode as the 50 MiB
        // production cap against an oversized archive, without moving 50 MiB
        // through the test.
        let body = vec![0x41u8; 4 * 1024];
        let url = spawn_stub_server(body);
        let client = reqwest::Client::new();
        let response = client.get(url).send().await.unwrap();
        let err = read_body_capped(response, 1024)
            .await
            .expect_err("oversized body must be rejected");
        let msg = format!("{:#}", err);
        assert!(
            msg.contains("download cap"),
            "expected size-cap error, got: {}",
            msg
        );
    }

    #[tokio::test]
    async fn content_length_over_cap_is_rejected_immediately() {
        let url = spawn_stub_server(vec![0x41u8; 64]);
        let client = reqwest::Client::new();
        let response = client.get(url).send().await.unwrap();
        let err = read_body_capped(response, 32)
            .await
            .expect_err("oversized Content-Length must be rejected");
        assert!(format!("{:#}", err).contains("download cap"));
    }

    #[tokio::test]
    async fn body_under_cap_is_returned_intact() {
        let body = vec![0x42u8; 512];
        let url = spawn_stub_server(body.clone());
        let client = reqwest::Client::new();
        let response = client.get(url).send().await.unwrap();
        let got = read_body_capped(response, 1024).await.unwrap();
        assert_eq!(got, body);
    }

    // sha256("abc"), FIPS 180-2 test vector.
    const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    #[test]
    fn sidecar_parses_sha256sum_output_case_insensitively() {
        let line = format!(
            "{}  par-term-web-frontend-v1.0.0.tar.gz\n",
            ABC_SHA256.to_uppercase()
        );
        assert_eq!(parse_sha256_sidecar(line.as_bytes()).unwrap(), ABC_SHA256);
        // Bare digest (no filename) is accepted too.
        assert_eq!(
            parse_sha256_sidecar(ABC_SHA256.as_bytes()).unwrap(),
            ABC_SHA256
        );
    }

    #[test]
    fn malformed_sidecar_is_an_error_not_a_skip() {
        for bad in [
            &b""[..],
            b"   \n",
            b"deadbeef  file.tar.gz",
            b"<html>Not Found</html>",
        ] {
            assert!(parse_sha256_sidecar(bad).is_err(), "accepted {:?}", bad);
        }
        let not_hex = "z".repeat(64);
        assert!(parse_sha256_sidecar(not_hex.as_bytes()).is_err());
    }

    #[test]
    fn archive_digest_match_passes_and_mismatch_refuses() {
        verify_archive_sha256(b"abc", ABC_SHA256).expect("matching digest must pass");
        let err = verify_archive_sha256(b"abd", ABC_SHA256).expect_err("tampered bytes must fail");
        assert!(format!("{:#}", err).contains("SHA-256 mismatch"));
    }

    #[tokio::test]
    async fn sidecar_fetch_round_trip_and_oversize_rejection() {
        let url = spawn_stub_server(format!("{ABC_SHA256}  a.tar.gz\n").into_bytes());
        let client = reqwest::Client::new();
        assert_eq!(
            fetch_expected_sha256(&client, &url).await.unwrap(),
            ABC_SHA256
        );

        let url = spawn_stub_server(vec![b'a'; MAX_SIDECAR_BYTES + 1]);
        assert!(fetch_expected_sha256(&client, &url).await.is_err());
    }

    #[test]
    fn missing_sidecar_warns_only_for_releases_before_the_cutoff() {
        let sidecar = "par-term-web-frontend-vX.tar.gz.sha256";
        for old in ["v0.58.0", "0.57.9", "v0.14.0"] {
            let warning = missing_sidecar_outcome(old, sidecar)
                .unwrap_or_else(|e| panic!("{old} predates sidecars, got error: {e:#}"));
            assert!(warning.contains("TLS only"), "{old}: {warning}");
        }
        for tag in [
            "v0.58.1",
            "v0.60.0",
            "v1.0.0",
            "v0.60.0-rc.1",
            "v0.60.0+build",
            "v0.60",
            "v0.60.0.1",
            "latest",
            "",
        ] {
            let err = missing_sidecar_outcome(tag, sidecar)
                .expect_err(&format!("{tag:?} without a sidecar must fail closed"));
            assert!(
                format!("{err:#}").contains("Refusing to extract"),
                "{tag:?}: {err:#}"
            );
        }
    }

    #[test]
    fn web_root_without_index_html_is_refused_unless_forced() {
        // A `TempDir` gives an OS-random, collision-proof base and removes
        // itself via `Drop` on both normal return and panic unwinding,
        // unlike the pid-derived path this replaced (a recycled pid could
        // collide across `cargo test` invocations, and a trailing
        // `remove_dir_all` never runs on a failed assertion).
        let base = tempfile::Builder::new()
            .prefix("par-term-webroot-test-")
            .tempdir()
            .expect("create temp dir for web root fixtures");

        // Directory without index.html → refused.
        let stray = base.path().join("stray");
        fs::create_dir_all(&stray).unwrap();
        fs::write(stray.join("important.txt"), "data").unwrap();
        let err =
            check_web_root_replaceable(&stray, false).expect_err("stray directory must be refused");
        assert!(format!("{:#}", err).contains("Refusing to replace web root"));
        // With force → allowed.
        check_web_root_replaceable(&stray, true).expect("force must allow replacement");

        // Directory with index.html (a previously extracted bundle) → allowed.
        let bundle = base.path().join("bundle");
        fs::create_dir_all(&bundle).unwrap();
        fs::write(bundle.join("index.html"), "<html></html>").unwrap();
        check_web_root_replaceable(&bundle, false).expect("bundle directory must be replaceable");

        // Nonexistent path → allowed (fresh install).
        check_web_root_replaceable(&base.path().join("fresh"), false)
            .expect("nonexistent web root must be allowed");
    }

    /// A minimal frontend-shaped archive: a regular file, an assets
    /// directory, and a nested regular file.
    fn append_regular_bundle(builder: &mut tar::Builder<Vec<u8>>) {
        let mut dir_header = tar::Header::new_gnu();
        dir_header.set_entry_type(tar::EntryType::dir());
        dir_header.set_size(0);
        dir_header.set_mode(0o755);
        builder
            .append_data(&mut dir_header, "assets", std::io::empty())
            .unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_mode(0o644);
        header.set_size(11);
        builder
            .append_data(&mut header, "assets/app.js", &b"console.log"[..])
            .unwrap();
        header.set_size(13);
        builder
            .append_data(&mut header, "index.html", &b"<html></html>"[..])
            .unwrap();
    }

    fn regular_bundle() -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        append_regular_bundle(&mut builder);
        builder.into_inner().unwrap()
    }

    #[test]
    fn regular_files_and_directories_extract() {
        let bytes = regular_bundle();
        let dest = tempfile::tempdir().expect("tempdir");
        let mut archive = Archive::new(bytes.as_slice());
        extract_regular_files(&mut archive, dest.path(), MAX_EXTRACTED_BYTES)
            .expect("regular bundle must extract");
        assert_eq!(
            fs::read(dest.path().join("assets/app.js")).unwrap(),
            b"console.log"
        );
        assert_eq!(
            fs::read(dest.path().join("index.html")).unwrap(),
            b"<html></html>"
        );
        assert!(dest.path().join("assets").is_dir());
        // Header permission bits are applied, matching `Archive::unpack`.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dest.path().join("assets/app.js"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o644);
        }
    }

    /// The same bundle plus one entry of a non-regular type, as a crafted
    /// release archive would carry it (SEC-006).
    fn bundle_with_entry(entry_type: tar::EntryType, name: &str) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        append_regular_bundle(&mut builder);
        if entry_type.is_hard_link() || entry_type.is_symlink() {
            let mut link_header = tar::Header::new_gnu();
            link_header.set_entry_type(entry_type);
            link_header.set_size(0);
            link_header.set_mode(0o777);
            builder
                .append_link(&mut link_header, name, "index.html")
                .unwrap();
        } else {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(entry_type);
            header.set_size(0);
            header.set_mode(0o644);
            builder
                .append_data(&mut header, name, std::io::empty())
                .unwrap();
        }
        builder.into_inner().unwrap()
    }

    #[test]
    fn symlink_entry_is_rejected() {
        let bytes = bundle_with_entry(tar::EntryType::symlink(), "evil-link");
        let dest = tempfile::tempdir().expect("tempdir");
        let mut archive = Archive::new(bytes.as_slice());
        let err = extract_regular_files(&mut archive, dest.path(), MAX_EXTRACTED_BYTES)
            .expect_err("a symlink entry must be refused");
        let msg = format!("{err:#}");
        assert!(msg.contains("evil-link"), "{msg}");
        assert!(
            !dest.path().join("evil-link").exists(),
            "no link may reach the web root"
        );
        // Extraction stops loudly at the hostile entry.
        assert!(dest.path().join("index.html").exists());
    }

    #[test]
    fn hardlink_entry_is_rejected() {
        let bytes = bundle_with_entry(tar::EntryType::hard_link(), "hard-link");
        let dest = tempfile::tempdir().expect("tempdir");
        let mut archive = Archive::new(bytes.as_slice());
        let err = extract_regular_files(&mut archive, dest.path(), MAX_EXTRACTED_BYTES)
            .expect_err("a hard-link entry must be refused");
        let msg = format!("{err:#}");
        assert!(msg.contains("hard-link"), "{msg}");
        assert!(!dest.path().join("hard-link").exists());
    }

    #[test]
    fn extraction_exceeding_total_cap_is_refused() {
        // CWE-409: MAX_ARCHIVE_BYTES bounds the compressed download only.
        // Two modest entries past a small cap must abort extraction; a
        // header lying about its size cannot push more than cap+1 bytes to
        // disk, because each entry is read through `take(remaining + 1)`.
        let mut builder = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_mode(0o644);
        header.set_size(1024);
        builder
            .append_data(&mut header, "a.txt", vec![0u8; 1024].as_slice())
            .unwrap();
        header.set_size(1024);
        builder
            .append_data(&mut header, "b.txt", vec![0u8; 1024].as_slice())
            .unwrap();
        let bytes = builder.into_inner().unwrap();
        let dest = tempfile::tempdir().expect("tempdir");
        let mut archive = Archive::new(bytes.as_slice());
        let err = extract_regular_files(&mut archive, dest.path(), 1500)
            .expect_err("a bundle past the extraction cap must be refused");
        let msg = format!("{err:#}");
        assert!(msg.contains("extraction cap"), "{msg}");
        // The overshooting entry must not have been fully written: at most
        // cap+1 bytes of b.txt reached disk.
        let written = fs::read(dest.path().join("b.txt")).unwrap().len() as u64;
        assert!(written <= 1501 - 1024, "b.txt wrote {written} bytes");
    }

    #[test]
    fn escaping_entry_path_is_rejected() {
        for path in ["../evil", "/etc/passwd", "a/../../b", ""] {
            assert!(safe_entry_path(Path::new(path)).is_err(), "{path:?}");
        }
        assert_eq!(
            safe_entry_path(Path::new("a/b.js")).unwrap(),
            Path::new("a/b.js")
        );
    }
}
