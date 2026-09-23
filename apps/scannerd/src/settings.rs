//! Settings changed on the page, kept beside the spool.
//!
//! The env file and the command line say how scannerd starts; the setup page
//! says what the person at the camera chose since, and wins. Kept as TOML in the
//! spool directory — the one place the unit file lets the daemon write — with
//! the token in it, so the file is readable by its owner only, like the env file.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::folders::Folder;
use crate::straighten::Corners;

const FILE: &str = "settings.toml";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// `x,y,w,h`, or empty for no crop at all — which is different from not
    /// set, where the env file's crop applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roi: Option<String>,
    /// The four corners of a page on the table, `x,y` each, top left first
    /// and clockwise — for a camera at an angle. Empty for none. When set,
    /// the crop is the area they span, whatever `roi` says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corners: Option<String>,
    /// Degrees clockwise to turn every photograph: 0, 90, 180 or 270 — for a
    /// camera mounted so that letters do not read upright.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotate: Option<u16>,
    /// The folders on the shelf, in the order the display prefers them. An
    /// empty list means none — not following letters at all — which is
    /// different from not set, where the env file's apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folders: Option<Vec<Folder>>,
    /// The exposure photographs are taken at, in stops from the camera's own
    /// choice. Learnt from the photographs and kept here, so a restart does
    /// not have to learn it again from the next page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ev: Option<f32>,
}

impl Settings {
    pub fn path(spool_dir: &Path) -> PathBuf {
        spool_dir.join(FILE)
    }

    /// The saved settings, or none when nothing has been saved yet.
    pub fn load(spool_dir: &Path) -> Result<Self> {
        let path = Self::path(spool_dir);
        match fs::read_to_string(&path) {
            Ok(text) => {
                toml::from_str(&text).with_context(|| format!("{} is not readable", path.display()))
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err).with_context(|| format!("could not read {}", path.display())),
        }
    }

    /// Written whole and renamed, and owner-only from the start: a token must
    /// not spend a moment world-readable while its permissions are fixed.
    pub fn save(&self, spool_dir: &Path) -> Result<()> {
        let path = Self::path(spool_dir);
        let partial = path.with_extension("toml.partial");
        let text = toml::to_string(self).context("could not write the settings")?;

        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options
            .open(&partial)
            .with_context(|| format!("could not write {}", partial.display()))?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        fs::rename(&partial, &path).with_context(|| format!("could not save {}", path.display()))
    }

    /// Takes what `newer` sets and keeps what it leaves out.
    ///
    /// A blank token keeps the one there is: the page never shows the token,
    /// so it has nothing to send back, and saving the crop must not log the
    /// scanner out of Paperless. A blank address is ignored for the same
    /// reason. An empty crop, though, means no crop.
    pub fn merge(&mut self, newer: Settings) {
        if let Some(url) = newer.url.filter(|url| !url.trim().is_empty()) {
            self.url = Some(url.trim().trim_end_matches('/').to_string());
        }
        if let Some(token) = newer.token.filter(|token| !token.trim().is_empty()) {
            self.token = Some(token.trim().to_string());
        }
        if let Some(tags) = newer.tags {
            self.tags = Some(
                tags.into_iter()
                    .map(|tag| tag.trim().to_string())
                    .filter(|tag| !tag.is_empty())
                    .collect(),
            );
        }
        if let Some(roi) = newer.roi {
            self.roi = Some(roi.trim().to_string());
        }
        if let Some(corners) = newer.corners {
            self.corners = Some(corners.trim().to_string());
        }
        if let Some(rotate) = newer.rotate {
            self.rotate = Some(rotate);
        }
        if let Some(ev) = newer.ev {
            self.ev = Some(ev);
        }
        if let Some(folders) = newer.folders {
            self.folders = Some(
                folders
                    .into_iter()
                    .map(|folder| Folder {
                        name: folder.name.trim().to_string(),
                        words: folder.words.trim().to_string(),
                        discard: folder.discard,
                    })
                    .filter(|folder| !folder.name.is_empty())
                    .collect(),
            );
        }
    }

    /// The folders in force: the page's, or the env file's.
    pub fn effective_folders(&self, started_with: &[Folder]) -> Vec<Folder> {
        self.folders
            .clone()
            .unwrap_or_else(|| started_with.to_vec())
    }

    /// Whether what came from the page can be used at all.
    pub fn validate(&self) -> Result<()> {
        if let Some(url) = self.url.as_deref().filter(|url| !url.trim().is_empty()) {
            let url = url.trim();
            if !url.starts_with("http://") && !url.starts_with("https://") {
                bail!("the Paperless address must start with http:// or https://");
            }
        }
        if let Some(roi) = self.roi.as_deref().filter(|roi| !roi.trim().is_empty()) {
            valid_roi(roi)?;
        }
        if let Some(corners) = self.corners.as_deref().filter(|c| !c.trim().is_empty()) {
            Corners::parse(corners)?;
        }
        if let Some(rotate) = self.rotate {
            valid_rotation(rotate)?;
        }
        if let Some(folders) = &self.folders {
            valid_folders(folders)?;
        }
        if let Some(ev) = self.ev {
            if !(crate::quality::EV_DARKEST..=crate::quality::EV_BRIGHTEST).contains(&ev) {
                bail!(
                    "the exposure is between {} and {} stops",
                    crate::quality::EV_DARKEST,
                    crate::quality::EV_BRIGHTEST
                );
            }
        }
        Ok(())
    }

    /// The crop to use: the page's if it set one (empty meaning none), or the
    /// one scannerd was started with.
    pub fn effective_roi(&self, started_with: Option<&str>) -> Option<String> {
        match self.roi.as_deref() {
            Some(roi) if roi.trim().is_empty() => None,
            Some(roi) => Some(roi.to_string()),
            None => started_with.map(str::to_string),
        }
    }
}

/// The crop and the corners in force.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct View {
    /// `x,y,w,h`: what the camera crops to.
    pub roi: Option<String>,
    /// The page's corners as fractions of the whole view, when the camera is
    /// at an angle.
    pub corners: Option<Corners>,
    /// Degrees clockwise every photograph is turned.
    pub rotate: u16,
}

impl View {
    /// The corners as fractions of the cropped photograph.
    pub fn corners_in_crop(&self) -> Option<Corners> {
        self.corners.as_ref()?.within(self.roi.as_deref()?)
    }

    /// What straightening is given: the corners in the photograph, started as
    /// far round as the turn says — or `None` when a photograph is to be kept
    /// as the camera took it.
    pub fn warp(&self) -> Option<Corners> {
        let quarters = self.rotate / 90;
        match self.corners_in_crop() {
            Some(corners) => Some(corners.turned(quarters)),
            None if !quarters.is_multiple_of(4) => Some(Corners::WHOLE.turned(quarters)),
            None => None,
        }
    }
}

impl Settings {
    /// What the camera looks at: the page's choice if it made one — a crop
    /// or corners, which it always saves together — or else what scannerd was
    /// started with. Corners decide the crop when there are any.
    /// The turn is separate: the page's if it chose one, else the start's.
    pub fn effective_view(
        &self,
        started_roi: Option<&str>,
        started_corners: Option<&str>,
        started_rotate: Option<u16>,
    ) -> Result<View> {
        let rotate = self.rotate.or(started_rotate).unwrap_or(0);
        valid_rotation(rotate)?;
        let corners = if self.roi.is_some() || self.corners.is_some() {
            self.corners.as_deref()
        } else {
            started_corners
        };
        match corners.map(str::trim).filter(|c| !c.is_empty()) {
            Some(corners) => {
                let corners = Corners::parse(corners)?;
                Ok(View {
                    roi: Some(corners.crop()),
                    corners: Some(corners),
                    rotate,
                })
            }
            None => Ok(View {
                roi: self.effective_roi(started_roi),
                corners: None,
                rotate,
            }),
        }
    }
}

/// Folders a person can tell apart, and at most one bin. Each is a Paperless
/// tag, and Paperless does not tell names apart by case.
pub fn valid_folders(folders: &[Folder]) -> Result<()> {
    if folders.len() > 20 {
        bail!("twenty folders at most");
    }
    let mut seen = std::collections::HashSet::new();
    for folder in folders {
        let name = folder.name.trim();
        if name.chars().count() > 60 {
            bail!("the folder name {name:?} is too long for a tag");
        }
        if !name.is_empty() && !seen.insert(name.to_lowercase()) {
            bail!("there are two folders called {name:?}");
        }
    }
    // Paperless refuses more than this, and would refuse it when the folders
    // are set up there — long after the words were saved here.
    for folder in folders {
        let length = folder.paperless_match().chars().count();
        if length > crate::folders::MATCH_LIMIT {
            bail!(
                "the words for {:?} are {length} characters; Paperless takes {} — leave some out",
                folder.name.trim(),
                crate::folders::MATCH_LIMIT
            );
        }
    }
    if folders.iter().filter(|folder| folder.discard).count() > 1 {
        bail!("only one folder can be the bin");
    }
    Ok(())
}

/// A quarter turn, a half or three quarters, or none.
pub fn valid_rotation(degrees: u16) -> Result<()> {
    if !matches!(degrees, 0 | 90 | 180 | 270) {
        bail!("a photograph turns by 0, 90, 180 or 270 degrees, not {degrees}");
    }
    Ok(())
}

/// `x,y,w,h`, each a fraction of the sensor, with the crop inside it.
pub fn valid_roi(roi: &str) -> Result<()> {
    let parts: Vec<f64> = roi
        .split(',')
        .map(|part| part.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .map_err(|_| anyhow::anyhow!("a crop is four numbers: x,y,w,h"))?;
    let [x, y, w, h] = parts[..] else {
        bail!("a crop is four numbers: x,y,w,h");
    };
    if [x, y, w, h].iter().any(|v| !(0.0..=1.0).contains(v)) {
        bail!("each part of a crop is a fraction between 0 and 1");
    }
    if w <= 0.0 || h <= 0.0 {
        bail!("a crop needs a width and a height");
    }
    // A little slack for the page's rounding to three places.
    if x + w > 1.0005 || y + h > 1.0005 {
        bail!("the crop runs off the edge of the picture");
    }
    Ok(())
}
