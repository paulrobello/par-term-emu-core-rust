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

/// Download and extract the web frontend from GitHub releases
pub async fn download_frontend(version: &str, web_root: &str) -> Result<()> {
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

    let archive_bytes = response
        .bytes()
        .await
        .context("Failed to read archive content")?;

    println!("Downloaded {} bytes", archive_bytes.len());

    // Create web root directory if it doesn't exist
    let web_root_path = Path::new(web_root);
    if web_root_path.exists() {
        println!("Clearing existing web root: {}", web_root);
        fs::remove_dir_all(web_root_path)
            .context(format!("Failed to remove existing directory: {}", web_root))?;
    }
    fs::create_dir_all(web_root_path)
        .context(format!("Failed to create web root directory: {}", web_root))?;

    // Extract the tar.gz archive
    println!("Extracting to: {}", web_root);
    let tar_gz = GzDecoder::new(archive_bytes.as_ref());
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
