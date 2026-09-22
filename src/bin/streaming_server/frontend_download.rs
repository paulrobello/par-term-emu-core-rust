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
const MAX_ARCHIVE_BYTES: usize = 50 * 1024 * 1024;

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

    archive
        .unpack(web_root_path)
        .context("Failed to extract archive")?;

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

    #[test]
    fn web_root_without_index_html_is_refused_unless_forced() {
        let base =
            std::env::temp_dir().join(format!("par-term-webroot-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);

        // Directory without index.html → refused.
        let stray = base.join("stray");
        fs::create_dir_all(&stray).unwrap();
        fs::write(stray.join("important.txt"), "data").unwrap();
        let err =
            check_web_root_replaceable(&stray, false).expect_err("stray directory must be refused");
        assert!(format!("{:#}", err).contains("Refusing to replace web root"));
        // With force → allowed.
        check_web_root_replaceable(&stray, true).expect("force must allow replacement");

        // Directory with index.html (a previously extracted bundle) → allowed.
        let bundle = base.join("bundle");
        fs::create_dir_all(&bundle).unwrap();
        fs::write(bundle.join("index.html"), "<html></html>").unwrap();
        check_web_root_replaceable(&bundle, false).expect("bundle directory must be replaceable");

        // Nonexistent path → allowed (fresh install).
        check_web_root_replaceable(&base.join("fresh"), false)
            .expect("nonexistent web root must be allowed");

        let _ = fs::remove_dir_all(&base);
    }
}
