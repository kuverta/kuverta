//! Whether a newer kuverta has been released.
//!
//! Asks GitHub for the repository's latest release and compares its tag with
//! the running version. It only tells: downloading and installing stay with
//! the person, from the release page, so nothing replaces the app behind
//! their back and no update signing key is needed yet.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{Result, RpcError};

/// Where releases are published. `KUVERTA_UPDATE_REPO` points elsewhere, for
/// a fork.
pub const DEFAULT_REPO: &str = "kuverta/kuverta";

/// What the window needs to offer an update.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UpdateInfo {
    pub current: String,
    pub latest: String,
    /// Whether `latest` is newer than `current`.
    pub newer: bool,
    /// The release page.
    pub url: String,
    /// The macOS disk image, when the release has one.
    pub download_url: Option<String>,
    pub notes: String,
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

pub fn repo() -> String {
    repo_from(std::env::var("KUVERTA_UPDATE_REPO").ok().as_deref())
}

/// `owner/name`, or the default for anything that is not one.
pub fn repo_from(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|repo| {
            let parts: Vec<&str> = repo.split('/').collect();
            parts.len() == 2
                && parts.iter().all(|part| {
                    !part.is_empty()
                        && part != &".."
                        && part
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                })
        })
        .map_or_else(|| DEFAULT_REPO.to_string(), str::to_string)
}

/// `1.2.3` from `v1.2.3`, `1.2.3-beta.1` and the like, as numbers; `None` for
/// a tag that is not a version.
pub fn parse_version(text: &str) -> Option<(Vec<u64>, bool)> {
    let text = text.trim().trim_start_matches(['v', 'V']);
    // A `-` starts a pre-release; a `+` only build metadata.
    let (numbers, pre) = match text.find(['-', '+']) {
        Some(at) => (&text[..at], text[at..].starts_with('-')),
        None => (text, false),
    };
    let parts: Vec<u64> = numbers
        .split('.')
        .map(|part| part.parse().ok())
        .collect::<Option<_>>()?;
    (!parts.is_empty() && parts.len() <= 4).then_some((parts, pre))
}

/// Whether `latest` is a newer version than `current`. A pre-release of the
/// same numbers is older than the release itself.
pub fn is_newer(current: &str, latest: &str) -> bool {
    let (Some((mut have, have_pre)), Some((mut get, get_pre))) =
        (parse_version(current), parse_version(latest))
    else {
        return false;
    };
    let width = have.len().max(get.len());
    have.resize(width, 0);
    get.resize(width, 0);
    match get.cmp(&have) {
        std::cmp::Ordering::Greater => true,
        std::cmp::Ordering::Less => false,
        std::cmp::Ordering::Equal => have_pre && !get_pre,
    }
}

fn read(current: &str, release: Release) -> Option<UpdateInfo> {
    if release.draft || release.prerelease {
        return None;
    }
    parse_version(&release.tag_name)?;
    let download_url = release
        .assets
        .iter()
        .find(|asset| asset.name.ends_with(".dmg"))
        .map(|asset| asset.browser_download_url.clone());
    Some(UpdateInfo {
        current: current.to_string(),
        newer: is_newer(current, &release.tag_name),
        latest: release.tag_name.trim_start_matches(['v', 'V']).to_string(),
        url: release.html_url,
        download_url,
        notes: release.body.unwrap_or_default(),
    })
}

/// The latest release of `repo`, compared with `current`. `Ok(None)` when the
/// repository has no release yet.
pub async fn check(repo: &str, current: &str) -> Result<Option<UpdateInfo>> {
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        // GitHub refuses requests without one.
        .user_agent(format!("kuverta/{current}"))
        .build()
        .map_err(|err| RpcError::Network(err.to_string()))?;
    let response = http
        .get(format!(
            "https://api.github.com/repos/{repo}/releases/latest"
        ))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|err| RpcError::Network(format!("could not ask GitHub for updates: {err}")))?;
    match response.status().as_u16() {
        200 => {}
        // No release yet — or a private repository, which looks the same.
        404 => return Ok(None),
        403 | 429 => {
            return Err(RpcError::Network(
                "GitHub is limiting requests for now; kuverta will ask again later".into(),
            ))
        }
        status => {
            return Err(RpcError::Network(format!(
                "GitHub answered {status} when asked for updates"
            )))
        }
    }
    let release: Release = response
        .json()
        .await
        .map_err(|err| RpcError::Rejected(format!("GitHub's answer was not a release: {err}")))?;
    Ok(read(current, release))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers() {
        assert!(is_newer("0.1.0", "v0.2.0"));
        assert!(is_newer("0.9.0", "0.10.0"), "not as text");
        assert!(is_newer("1.2", "1.2.1"));
        assert!(!is_newer("0.2.0", "v0.2.0"));
        assert!(!is_newer("0.3.0", "v0.2.9"));
        assert!(
            is_newer("0.2.0-beta.1", "0.2.0"),
            "the release beats its beta"
        );
        assert!(!is_newer("0.2.0", "0.2.0-beta.1"));
        assert!(!is_newer("0.1.0", "nightly"));
        assert!(!is_newer("0.1.0", ""));
        assert!(
            !is_newer("0.2.0", "0.2.0+build.7"),
            "build metadata is not newer"
        );
    }

    fn release(tag: &str) -> Release {
        serde_json::from_value(serde_json::json!({
            "tag_name": tag,
            "html_url": format!("https://github.com/kuverta/kuverta/releases/tag/{tag}"),
            "body": "Fixes.",
            "draft": false,
            "prerelease": false,
            "assets": [
                { "name": "kuverta_0.2.0_universal.app.tar.gz", "browser_download_url": "https://example/app.tar.gz" },
                { "name": "kuverta_0.2.0_universal.dmg", "browser_download_url": "https://example/kuverta.dmg" }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn a_release_says_where_its_disk_image_is() {
        let info = read("0.1.0", release("v0.2.0")).unwrap();
        assert!(info.newer);
        assert_eq!(info.latest, "0.2.0");
        assert_eq!(
            info.download_url.as_deref(),
            Some("https://example/kuverta.dmg")
        );
        assert!(info.url.ends_with("/v0.2.0"));

        let same = read("0.2.0", release("v0.2.0")).unwrap();
        assert!(!same.newer);

        let mut beta = release("v0.3.0-beta.1");
        beta.prerelease = true;
        assert_eq!(read("0.2.0", beta), None, "pre-releases are not offered");
        assert_eq!(read("0.2.0", release("latest-build")), None);
    }

    #[test]
    fn the_repository_can_be_pointed_elsewhere_but_only_at_a_repository() {
        assert_eq!(repo_from(None), DEFAULT_REPO);
        assert_eq!(repo_from(Some(" someone/fork ")), "someone/fork");
        assert_eq!(repo_from(Some("../x")), DEFAULT_REPO);
        assert_eq!(repo_from(Some("a/b/c")), DEFAULT_REPO);
        assert_eq!(repo_from(Some("not a/repo")), DEFAULT_REPO);
    }
}
