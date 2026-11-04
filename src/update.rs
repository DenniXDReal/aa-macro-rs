//! Auto-update from the repo's `latest` GitHub release.
//!
//! Every CI build bakes its commit (`AA_BUILD_SHA`) and run number
//! (`AA_BUILD_NUM`) into the exe and publishes a release whose body says
//! `Built from <sha> (build <n>)`. The app compares that sha with its own.
//!
//! Updating: download the zip, write `aa-macro.new.exe`, rename the running
//! exe to `aa-macro.old.exe` (Windows allows renaming a running exe, not
//! overwriting it), move the new one into place, restart. The old file is
//! deleted on the next start.
//!
//! The repo is private, so a fine-grained GitHub token with read-only
//! "Contents" access to this one repo is needed (stored DPAPI-encrypted).

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const BUILD_SHA: &str = match option_env!("AA_BUILD_SHA") {
    Some(s) => s,
    None => "dev",
};
/// Release tag this build updates from ("latest", or "parallel" for the
/// parallel-checks test build).
pub const CHANNEL: &str = match option_env!("AA_CHANNEL") {
    Some(c) => c,
    None => "latest",
};

pub const BUILD_NUM: &str = match option_env!("AA_BUILD_NUM") {
    Some(s) => s,
    None => "dev",
};
pub const ASSET: &str = "aa-macro-windows.zip";
pub const TOKEN_KEY: &str = "github_token";

pub fn version_label() -> String {
    let short = &BUILD_SHA[..BUILD_SHA.len().min(7)];
    format!(
        "v{} build {} ({}){}",
        env!("CARGO_PKG_VERSION"),
        BUILD_NUM,
        short,
        if CHANNEL == "latest" {
            ""
        } else {
            " · PARALLEL TEST"
        }
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Available {
    pub sha: String,
    pub build: String,
    pub asset_api_url: String,
    pub published: String,
}

#[derive(Deserialize)]
struct Release {
    body: Option<String>,
    published_at: Option<String>,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    url: String,
}

/// `Built from <40 hex> (build 12)` -> (sha, "12").
pub fn parse_body(body: &str) -> Option<(String, String)> {
    let sha = body
        .split(|c: char| !c.is_ascii_hexdigit())
        .find(|w| w.len() == 40)?
        .to_ascii_lowercase();
    let build = body
        .split("build ")
        .nth(1)
        .map(|r| {
            r.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "?".into());
    Some((sha, build))
}

/// HTTP agent that trusts the OS certificate store (works behind antivirus
/// HTTPS scanning, which Mozilla's bundled roots would reject).
fn agent() -> ureq::Agent {
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    ureq::Agent::config_builder()
        .tls_config(tls)
        .build()
        .new_agent()
}

fn get(url: &str, token: &str, accept: &str) -> Result<Vec<u8>> {
    let mut resp = agent()
        .get(url)
        .header("Authorization", &format!("Bearer {}", token.trim()))
        .header("Accept", accept)
        .header("User-Agent", "aa-macro-updater")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .map_err(|e| match e {
            ureq::Error::StatusCode(401) => {
                anyhow!("GitHub rejected the token (401) - make a new one")
            }
            ureq::Error::StatusCode(404) => {
                anyhow!("release not found (404) - token needs read access to the repo")
            }
            e => anyhow!("download failed: {e}"),
        })?;
    Ok(resp
        .body_mut()
        .with_config()
        .limit(200 * 1024 * 1024)
        .read_to_vec()?)
}

/// `Some` when the `latest` release was built from a different commit.
pub fn check(repo: &str, token: &str) -> Result<Option<Available>> {
    let url = format!("https://api.github.com/repos/{repo}/releases/tags/{CHANNEL}");
    let rel: Release = serde_json::from_slice(&get(&url, token, "application/vnd.github+json")?)?;
    let (sha, build) = parse_body(rel.body.as_deref().unwrap_or(""))
        .ok_or_else(|| anyhow!("release has no build info"))?;
    let asset = rel
        .assets
        .iter()
        .find(|a| a.name == ASSET)
        .ok_or_else(|| anyhow!("release has no {ASSET}"))?;
    if sha.eq_ignore_ascii_case(BUILD_SHA) {
        return Ok(None);
    }
    Ok(Some(Available {
        sha,
        build,
        asset_api_url: asset.url.clone(),
        published: rel.published_at.unwrap_or_default(),
    }))
}

/// Download and install. Returns the path of the new exe (restart it).
pub fn install(a: &Available, token: &str) -> Result<PathBuf> {
    let zip_bytes = get(&a.asset_api_url, token, "application/octet-stream")?;
    let exe = std::env::current_exe()?;
    let dir = exe
        .parent()
        .ok_or_else(|| anyhow!("exe has no folder"))?
        .to_path_buf();
    install_from_zip(&zip_bytes, &exe, &dir)
}

/// Unpack `aa-macro.exe` (+ README) from the zip and swap it in for `exe`.
pub fn install_from_zip(zip_bytes: &[u8], exe: &Path, dir: &Path) -> Result<PathBuf> {
    let mut zip =
        zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).context("bad update zip")?;
    let mut new_exe = None;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i)?;
        let full = f.name()?.to_string();
        let name = full.rsplit(['/', '\\']).next().unwrap_or("").to_string();
        let mut data = vec![];
        f.read_to_end(&mut data)?;
        match name.as_str() {
            "aa-macro.exe" => new_exe = Some(data),
            "README.md" => std::fs::write(dir.join("README.md"), data)?,
            _ => {}
        }
    }
    let data = new_exe.ok_or_else(|| anyhow!("zip has no aa-macro.exe"))?;
    if data.len() < 100_000 {
        bail!("downloaded exe looks broken ({} bytes)", data.len());
    }
    let staged = dir.join("aa-macro.new.exe");
    let old = dir.join("aa-macro.old.exe");
    std::fs::write(&staged, &data)?;
    let _ = std::fs::remove_file(&old);
    std::fs::rename(exe, &old).context("could not move the running exe aside")?;
    if let Err(e) = std::fs::rename(&staged, exe) {
        // Put the old one back so the user is never left without an exe.
        let _ = std::fs::rename(&old, exe);
        return Err(e).context("could not move the new exe into place");
    }
    Ok(exe.to_path_buf())
}

/// Delete the previous exe left by an update (call at startup).
pub fn cleanup_old() {
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        let _ = std::fs::remove_file(dir.join("aa-macro.old.exe"));
        let _ = std::fs::remove_file(dir.join("aa-macro.new.exe"));
    }
}

/// Start the (new) exe with the same arguments; the caller then exits.
pub fn restart(exe: &Path) -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::Command::new(exe)
        .args(args)
        .spawn()
        .context("restarting")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_release_body() {
        let b = "Built from 4587F7B2A4517C70E88EAB588DF8868C515EDE5B (build 17). Download it.";
        assert_eq!(
            parse_body(b),
            Some((
                "4587f7b2a4517c70e88eab588df8868c515ede5b".into(),
                "17".into()
            ))
        );
        assert_eq!(parse_body("no sha here"), None);
        let old = "Built from 4587f7b2a4517c70e88eab588df8868c515ede5b. Download";
        assert_eq!(parse_body(old).unwrap().1, "?");
    }

    #[test]
    fn swaps_exe_from_zip() {
        let dir = std::env::temp_dir().join(format!("aa-upd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("aa-macro.exe");
        std::fs::write(&exe, b"old build").unwrap();

        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("aa-macro.exe", o).unwrap();
            z.write_all(&vec![7u8; 200_000]).unwrap();
            z.start_file("README.md", o).unwrap();
            z.write_all(b"# new readme").unwrap();
            z.finish().unwrap();
        }
        let p = install_from_zip(buf.get_ref(), &exe, &dir).unwrap();
        assert_eq!(p, exe);
        assert_eq!(std::fs::read(&exe).unwrap().len(), 200_000);
        assert_eq!(
            std::fs::read(dir.join("aa-macro.old.exe")).unwrap(),
            b"old build"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("README.md")).unwrap(),
            "# new readme"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_zip_without_exe() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            z.start_file("README.md", zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(b"x").unwrap();
            z.finish().unwrap();
        }
        let dir = std::env::temp_dir();
        assert!(install_from_zip(buf.get_ref(), &dir.join("nope.exe"), &dir).is_err());
    }
}
