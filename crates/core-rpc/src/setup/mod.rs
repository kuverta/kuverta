//! Setting kuverta up on a new computer.
//!
//! What the setup assistant asks about, as calls it can make: whether the
//! local model server is installed and running and has the models the jobs
//! use; whether a Paperless is reachable, or can be installed with Docker;
//! which mail accounts other programs on this computer already know; and
//! whether setup has been done at all.
//!
//! Nothing here installs software on its own. Ollama and Docker are the
//! person's to install, from their makers, and this only says where. The one
//! thing it runs is Paperless, in Docker, when asked to — a compose file
//! written into kuverta's own data directory, which is where it can be found
//! and removed again.

pub mod apple_mail;
pub mod autoconfig;
pub mod passwords;
pub mod thunderbird;

use std::io::{BufRead, BufReader};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub use core_ai::PullProgress;

use crate::settings::AccountInput;
use crate::{Core, Result, RpcError, Task};

// -- whether setup has run ------------------------------------------------------

const MARKER: &str = "setup-finished";

/// Whether the assistant should open by itself.
#[derive(Debug, Clone, Serialize)]
pub struct SetupStatus {
    /// Setup was never finished, and there is nothing yet to show.
    pub first_run: bool,
    pub finished: bool,
    pub platform: &'static str,
}

impl Core {
    /// Asked once when the window opens. An existing store with accounts in it
    /// is not a first run, marker or not: those are people who set kuverta up
    /// before there was an assistant.
    pub fn setup_status(&self, data_dir: &Path) -> Result<SetupStatus> {
        let finished = data_dir.join(MARKER).exists();
        let empty = self.store.accounts()?.is_empty() && self.store.paper_mailboxes()?.is_empty();
        Ok(SetupStatus {
            first_run: !finished && empty,
            finished,
            platform: platform(),
        })
    }
}

/// Remembers that the assistant was finished or put aside, so it does not
/// open again on its own.
pub fn finish(data_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(data_dir)?;
    std::fs::write(data_dir.join(MARKER), now().to_string())?;
    Ok(())
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

pub fn platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

// -- finding programs -------------------------------------------------------------

/// Where programs are commonly installed, beyond `PATH`.
///
/// A program started from the Dock or Finder gets a `PATH` of
/// `/usr/bin:/bin:/usr/sbin:/sbin`, which has neither Homebrew nor Docker in
/// it, so looking only there would report both missing on most Macs.
fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    let mut extra = vec![
        PathBuf::from("/usr/local/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/bin"),
        PathBuf::from("/bin"),
        PathBuf::from("/Applications/Docker.app/Contents/Resources/bin"),
        PathBuf::from("/Applications/Ollama.app/Contents/Resources"),
        PathBuf::from("/snap/bin"),
    ];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        extra.push(home.join(".orbstack/bin"));
        extra.push(home.join(".local/bin"));
        extra.push(home.join(".docker/bin"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        extra.push(local.join("Programs/Ollama"));
    }
    if let Some(files) = std::env::var_os("ProgramFiles").map(PathBuf::from) {
        extra.push(files.join("Docker/Docker/resources/bin"));
    }
    for dir in extra {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    }
    dirs
}

pub fn find_program(name: &str) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{name}.exe"), name.to_string()]
    } else {
        vec![name.to_string()]
    };
    search_dirs()
        .into_iter()
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|path| path.is_file())
}

/// A `PATH` with every place looked in, for the programs this starts: Docker's
/// command line runs credential helpers that live beside it.
fn child_path() -> std::ffi::OsString {
    std::env::join_paths(search_dirs()).unwrap_or_default()
}

fn mac_app(name: &str) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let mut places = vec![PathBuf::from("/Applications")];
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        places.push(home.join("Applications"));
    }
    places
        .into_iter()
        .map(|dir| dir.join(format!("{name}.app")))
        .find(|app| app.is_dir())
}

/// How to get a program: a page to download it from, and a command for
/// those who would rather type one.
#[derive(Debug, Clone, Serialize)]
pub struct InstallHelp {
    pub url: String,
    pub command: Option<String>,
    pub note: Option<String>,
}

// -- the local model server ----------------------------------------------------------

/// A model a job needs, and whether it is there.
#[derive(Debug, Clone, Serialize)]
pub struct NeededModel {
    pub task: Task,
    pub model: String,
    pub present: bool,
    /// Roughly how much a download is, where known.
    pub size: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OllamaStatus {
    pub base_url: String,
    /// Whether the address is this computer. An Ollama elsewhere is not this
    /// computer's to install or start.
    pub local: bool,
    pub installed: bool,
    pub running: bool,
    pub version: Option<String>,
    /// Why it is not answering, when it is not.
    pub problem: Option<String>,
    pub needed: Vec<NeededModel>,
    pub install: InstallHelp,
}

fn model_size(model: &str) -> Option<&'static str> {
    match model {
        "qwen2.5vl:3b" => Some("3.2 GB"),
        "qwen2.5vl:7b" => Some("6.0 GB"),
        "llama3.2:3b" => Some("2.0 GB"),
        "llama3.2:1b" => Some("1.3 GB"),
        _ => None,
    }
}

fn ollama_install() -> InstallHelp {
    match platform() {
        "macos" => InstallHelp {
            url: "https://ollama.com/download/mac".into(),
            command: Some("brew install ollama && brew services start ollama".into()),
            note: Some("Download it, move Ollama to Applications and open it once.".into()),
        },
        "windows" => InstallHelp {
            url: "https://ollama.com/download/windows".into(),
            command: None,
            note: Some("Run the installer; Ollama then starts with Windows.".into()),
        },
        _ => InstallHelp {
            url: "https://ollama.com/download/linux".into(),
            command: Some("curl -fsSL https://ollama.com/install.sh | sh".into()),
            note: Some("The script installs Ollama as a service that starts by itself.".into()),
        },
    }
}

/// What a local-model job would find, looked at with no lock held: the
/// provider and models are read from the store first, by
/// [`Core::ollama_setup`].
pub struct OllamaProbe {
    client: core_ai::Ollama,
    local: bool,
    needed: Vec<(Task, String)>,
}

impl Core {
    /// The Ollama the jobs use by default, and the models they are set to use
    /// on it.
    pub fn ollama_setup(&self) -> Result<OllamaProbe> {
        let provider = self
            .store
            .ai_provider(crate::ai::LOCAL_PROVIDER)?
            .ok_or_else(|| RpcError::Rejected("no local model server is configured".into()))?;
        let client = core_ai::Ollama::new(&provider.base_url)
            .map_err(|err| RpcError::Rejected(err.to_string()))?;
        let mut needed = Vec::new();
        for task in self.ai_tasks()? {
            let on_it = task.provider_id == crate::ai::LOCAL_PROVIDER;
            if on_it && !needed.iter().any(|(_, model)| model == &task.model) {
                needed.push((task.task, task.model));
            }
        }
        Ok(OllamaProbe {
            local: crate::ai::is_loopback(&provider.base_url),
            client,
            needed,
        })
    }
}

impl OllamaProbe {
    pub async fn status(&self) -> OllamaStatus {
        let installed = find_program("ollama").is_some() || mac_app("Ollama").is_some();
        let (running, version, problem) = match self.client.version().await {
            Ok(version) => (true, Some(version), None),
            Err(err) => (false, None, Some(err.to_string())),
        };
        let pulled: Vec<String> = if running {
            self.client
                .models()
                .await
                .map(|models| models.into_iter().map(|model| model.name).collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let has = |model: &str| {
            pulled.iter().any(|name| {
                name == model || (!model.contains(':') && name == &format!("{model}:latest"))
            })
        };
        OllamaStatus {
            base_url: self.client.base_url().to_string(),
            local: self.local,
            // An Ollama that answers is installed somewhere, whatever this
            // computer's directories say.
            installed: installed || running,
            running,
            version,
            problem,
            needed: self
                .needed
                .iter()
                .map(|(task, model)| NeededModel {
                    task: *task,
                    model: model.clone(),
                    present: has(model),
                    size: model_size(model),
                })
                .collect(),
            install: ollama_install(),
        }
    }

    pub async fn pull(
        &self,
        model: &str,
        progress: impl FnMut(core_ai::PullProgress),
    ) -> Result<()> {
        self.client
            .pull(model, progress)
            .await
            .map_err(|err| crate::ai::ai_error(self.client.base_url(), err))
    }

    /// Starts the Ollama on this computer and waits for it to answer.
    pub async fn start(&self) -> Result<()> {
        if !self.local {
            return Err(RpcError::Rejected(format!(
                "{} is not this computer; start Ollama there",
                self.client.base_url()
            )));
        }
        if self.client.version().await.is_ok() {
            return Ok(());
        }
        start_ollama()?;
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            if self.client.version().await.is_ok() {
                return Ok(());
            }
        }
        Err(RpcError::Network(
            "Ollama was started but has not answered after 20 seconds".into(),
        ))
    }
}

fn start_ollama() -> Result<()> {
    // The app on a Mac, which also keeps Ollama running after a restart.
    if let Some(app) = mac_app("Ollama") {
        Command::new("open")
            .arg("-a")
            .arg(app)
            .status()
            .map_err(|err| RpcError::Rejected(format!("could not open Ollama: {err}")))?;
        return Ok(());
    }
    let program = find_program("ollama")
        .ok_or_else(|| RpcError::Rejected("Ollama is not installed on this computer".into()))?;
    Command::new(program)
        .arg("serve")
        .env("PATH", child_path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| RpcError::Rejected(format!("could not start Ollama: {err}")))?;
    Ok(())
}

// -- Docker and Paperless --------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct DockerStatus {
    pub installed: bool,
    pub running: bool,
    pub version: Option<String>,
    pub problem: Option<String>,
    pub install: InstallHelp,
}

fn docker_install() -> InstallHelp {
    match platform() {
        "macos" => InstallHelp {
            url: "https://docs.docker.com/desktop/setup/install/mac-install/".into(),
            command: None,
            note: Some(
                "Docker Desktop, or OrbStack (orbstack.dev), which is lighter; either works."
                    .into(),
            ),
        },
        "windows" => InstallHelp {
            url: "https://docs.docker.com/desktop/setup/install/windows-install/".into(),
            command: None,
            note: None,
        },
        _ => InstallHelp {
            url: "https://docs.docker.com/engine/install/".into(),
            command: Some("curl -fsSL https://get.docker.com | sh".into()),
            note: Some("Then add yourself to the docker group and log in again.".into()),
        },
    }
}

/// Whether Docker is installed and its engine is running. Blocking: it runs
/// `docker info`.
pub fn docker_status() -> DockerStatus {
    let Some(docker) = find_program("docker") else {
        return DockerStatus {
            installed: mac_app("Docker").is_some() || mac_app("OrbStack").is_some(),
            running: false,
            version: None,
            problem: None,
            install: docker_install(),
        };
    };
    let output = Command::new(docker)
        .args(["info", "--format", "{{.ServerVersion}}"])
        .env("PATH", child_path())
        .stdin(Stdio::null())
        .output();
    let (running, version, problem) = match output {
        Ok(output) if output.status.success() => (
            true,
            Some(String::from_utf8_lossy(&output.stdout).trim().to_string()),
            None,
        ),
        Ok(output) => (
            false,
            None,
            Some(
                String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("the Docker engine is not running")
                    .trim()
                    .to_string(),
            ),
        ),
        Err(err) => (false, None, Some(err.to_string())),
    };
    DockerStatus {
        installed: true,
        running,
        version,
        problem,
        install: docker_install(),
    }
}

/// Opens Docker Desktop or OrbStack, whichever is installed.
pub fn start_docker() -> Result<()> {
    for app in ["Docker", "OrbStack"] {
        if let Some(app) = mac_app(app) {
            Command::new("open")
                .arg("-a")
                .arg(app)
                .status()
                .map_err(|err| RpcError::Rejected(format!("could not open Docker: {err}")))?;
            return Ok(());
        }
    }
    Err(RpcError::Rejected(
        "start the Docker engine yourself — on Linux, `sudo systemctl start docker`".into(),
    ))
}

/// A Paperless setup found or knows about.
#[derive(Debug, Clone, Serialize)]
pub struct PaperlessFound {
    pub base_url: String,
    /// Already an address in kuverta.
    pub configured: bool,
    /// The one kuverta installed, whether or not it is running.
    pub installed_by_kuverta: bool,
    pub answering: bool,
}

/// Where Paperless is usually found, and where kuverta would have put it.
pub async fn find_paperless(configured: &[String], data_dir: &Path) -> Vec<PaperlessFound> {
    let ours = installed_paperless(data_dir);
    let mut candidates: Vec<String> = configured.to_vec();
    if let Some((url, _)) = &ours {
        candidates.push(url.clone());
    }
    for url in [
        "http://localhost:8000",
        "http://paperless.local:8000",
        "http://paperless:8000",
    ] {
        candidates.push(url.to_string());
    }
    let normal = |url: &str| url.trim().trim_end_matches('/').to_ascii_lowercase();

    let mut found: Vec<PaperlessFound> = Vec::new();
    for url in candidates {
        if found
            .iter()
            .any(|known| normal(&known.base_url) == normal(&url))
        {
            continue;
        }
        let answering = core_paper::probe(&url).await == core_paper::Probe::Paperless;
        let configured = configured.iter().any(|known| normal(known) == normal(&url));
        let installed_by_kuverta = ours
            .as_ref()
            .is_some_and(|(ours, _)| normal(ours) == normal(&url));
        if answering || configured || installed_by_kuverta {
            found.push(PaperlessFound {
                base_url: url,
                configured,
                installed_by_kuverta,
                answering,
            });
        }
    }
    found
}

/// What installing Paperless needs to know.
#[derive(Debug, Clone, Deserialize)]
pub struct PaperlessInstall {
    /// The first Paperless user, which kuverta then signs in as.
    pub username: String,
    pub password: String,
    /// Whether other devices on the network may reach it — the scanner has
    /// to, to deliver post.
    pub on_network: bool,
}

fn paperless_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("paperless")
}

/// The address of the Paperless kuverta installed, and its directory.
pub fn installed_paperless(data_dir: &Path) -> Option<(String, PathBuf)> {
    let dir = paperless_dir(data_dir);
    let port = std::fs::read_to_string(dir.join("port")).ok()?;
    let port: u16 = port.trim().parse().ok()?;
    dir.join("docker-compose.yml")
        .is_file()
        .then(|| (format!("http://localhost:{port}"), dir))
}

/// The pinned image, as the dev stack pins it: `latest` has changed what it
/// requires under people before.
const PAPERLESS_IMAGE: &str = "ghcr.io/paperless-ngx/paperless-ngx:3.1.3";

/// Never 8000: that is where the dev stack's Paperless listens, and an
/// installed Paperless on it would stop the dev stack from starting.
/// The ports an install tries, in order. A dev instance is a hundred above,
/// so a development build installing its own Paperless can never take the
/// port the installed app's is on — nor land on it when that one is stopped.
fn port_choices() -> [u16; 5] {
    match core_accounts::instance() {
        Some(_) => [8110, 8120, 8130, 8140, 8988],
        None => [8010, 8020, 8030, 8040, 8888],
    }
}

fn free_port() -> u16 {
    let choices = port_choices();
    choices
        .into_iter()
        .find(|&port| TcpListener::bind(("0.0.0.0", port)).is_ok())
        .unwrap_or(choices[0] + 40)
}

/// The compose project an install is: its containers, its network and its
/// volumes all hang off this name, so two kuvertas sharing it would be
/// starting and stopping each other's Paperless and writing to one database.
/// A dev instance therefore gets its own. The name people already have is
/// kept as it is: renaming it would leave its volumes — its post — behind.
fn compose_project(dir: &Path) -> String {
    if let Some(name) = std::fs::read_to_string(dir.join("docker-compose.yml"))
        .ok()
        .and_then(|text| {
            text.lines().find_map(|line| {
                line.strip_prefix("name:")
                    .map(|name| name.trim().to_string())
            })
        })
        .filter(|name| !name.is_empty())
    {
        return name;
    }
    match core_accounts::instance() {
        Some(instance) => format!("kuverta-paperless-{instance}"),
        None => "kuverta-paperless".to_string(),
    }
}

fn time_zone() -> String {
    if let Ok(zone) = std::env::var("TZ") {
        if !zone.trim().is_empty() {
            return zone.trim().trim_start_matches(':').to_string();
        }
    }
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|target| {
            let target = target.to_string_lossy().to_string();
            target
                .split_once("zoneinfo/")
                .map(|(_, zone)| zone.to_string())
        })
        .unwrap_or_else(|| "UTC".to_string())
}

fn secret_key() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|err| RpcError::Rejected(format!("no randomness for a secret key: {err}")))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Writes the compose file and its settings, keeping the port, key and
/// password of an earlier install, and returns the address.
pub fn write_paperless(data_dir: &Path, install: &PaperlessInstall) -> Result<(String, PathBuf)> {
    if install.username.trim().is_empty() || install.password.len() < 8 {
        return Err(RpcError::Rejected(
            "Paperless needs a user name and a password of at least eight characters".into(),
        ));
    }
    if install.username.contains(['\n', '=']) || install.password.contains('\n') {
        return Err(RpcError::Rejected(
            "the user name and password cannot contain line breaks, nor the name an =".into(),
        ));
    }
    let dir = paperless_dir(data_dir);
    let project = compose_project(&dir);
    std::fs::create_dir_all(dir.join("consume"))?;
    std::fs::create_dir_all(dir.join("export"))?;

    let port = match std::fs::read_to_string(dir.join("port")) {
        Ok(text) => text.trim().parse().unwrap_or_else(|_| free_port()),
        Err(_) => free_port(),
    };
    let env_path = dir.join("paperless.env");
    let previous_key = std::fs::read_to_string(&env_path).ok().and_then(|text| {
        text.lines()
            .find_map(|line| line.strip_prefix("PAPERLESS_SECRET_KEY="))
            .map(str::to_string)
    });
    let key = match previous_key {
        Some(key) => key,
        None => secret_key()?,
    };

    let bind = if install.on_network { "" } else { "127.0.0.1:" };
    let compose = format!(
        "# Paperless-ngx for kuverta, written by its setup assistant.\n\
         # Start: docker compose up -d    Stop: docker compose down\n\
         name: {project}\n\
         services:\n\
         \x20 broker:\n\
         \x20   image: docker.io/library/redis:7-alpine\n\
         \x20   restart: unless-stopped\n\
         \x20   volumes:\n\
         \x20     - redisdata:/data\n\
         \x20 webserver:\n\
         \x20   image: {PAPERLESS_IMAGE}\n\
         \x20   restart: unless-stopped\n\
         \x20   depends_on:\n\
         \x20     - broker\n\
         \x20   ports:\n\
         \x20     - \"{bind}{port}:8000\"\n\
         \x20   env_file: paperless.env\n\
         \x20   volumes:\n\
         \x20     - data:/usr/src/paperless/data\n\
         \x20     - media:/usr/src/paperless/media\n\
         \x20     - ./export:/usr/src/paperless/export\n\
         \x20     - ./consume:/usr/src/paperless/consume\n\
         volumes:\n\
         \x20 data:\n\
         \x20 media:\n\
         \x20 redisdata:\n"
    );
    // The admin password is only read when Paperless first creates the user;
    // it stays in this owner-only file so a reinstall creates the same one.
    let env = format!(
        "PAPERLESS_REDIS=redis://broker:6379\n\
         PAPERLESS_SECRET_KEY={key}\n\
         PAPERLESS_ADMIN_USER={}\n\
         PAPERLESS_ADMIN_PASSWORD={}\n\
         PAPERLESS_TIME_ZONE={}\n\
         PAPERLESS_OCR_LANGUAGE=deu+eng\n\
         PAPERLESS_OCR_DESKEW=true\n\
         PAPERLESS_OCR_ROTATE_PAGES=true\n",
        install.username.trim(),
        install.password,
        time_zone(),
    );
    std::fs::write(dir.join("docker-compose.yml"), compose)?;
    write_private(&env_path, &env)?;
    std::fs::write(dir.join("port"), port.to_string())?;
    Ok((format!("http://localhost:{port}"), dir))
}

fn write_private(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(text.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Runs `docker compose up -d` in `dir`, passing each line it prints to
/// `output`. Blocking, for as long as the images take to download.
pub fn compose_up(dir: &Path, mut output: impl FnMut(String)) -> Result<()> {
    let docker = find_program("docker")
        .ok_or_else(|| RpcError::Rejected("Docker is not installed".into()))?;
    let mut child = Command::new(docker)
        .args(["compose", "up", "-d"])
        .current_dir(dir)
        .env("PATH", child_path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| RpcError::Rejected(format!("could not run docker compose: {err}")))?;

    let (sender, receiver) = std::sync::mpsc::channel::<String>();
    let mut readers = Vec::new();
    for stream in [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn std::io::Read + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let sender = sender.clone();
        readers.push(std::thread::spawn(move || {
            // Docker redraws progress with carriage returns; each is a line.
            let mut reader = BufReader::new(stream);
            let mut buffer = Vec::new();
            while reader.read_until(b'\n', &mut buffer).unwrap_or(0) > 0 {
                for part in String::from_utf8_lossy(&buffer).split('\r') {
                    let line = part.trim();
                    if !line.is_empty() {
                        let _ = sender.send(line.to_string());
                    }
                }
                buffer.clear();
            }
        }));
    }
    drop(sender);
    let mut last = Vec::new();
    for line in receiver {
        last.push(line.clone());
        if last.len() > 5 {
            last.remove(0);
        }
        output(line);
    }
    for reader in readers {
        let _ = reader.join();
    }
    let status = child
        .wait()
        .map_err(|err| RpcError::Rejected(format!("docker compose did not finish: {err}")))?;
    if !status.success() {
        return Err(RpcError::Rejected(format!(
            "docker compose failed: {}",
            last.join(" / ")
        )));
    }
    Ok(())
}

/// Waits for a freshly started Paperless to answer, then signs in. A first
/// start sets up its database and takes a minute or two; the user it creates
/// arrives a little after the API does.
pub async fn wait_and_sign_in(
    base_url: &str,
    username: &str,
    password: &str,
    mut progress: impl FnMut(String),
) -> Result<String> {
    let mut waited = 0u64;
    loop {
        if core_paper::probe(base_url).await == core_paper::Probe::Paperless {
            match core_paper::obtain_token(base_url, username, password).await {
                Ok(token) => return Ok(token),
                Err(core_paper::PaperError::Auth) if waited < 600 => {}
                Err(err) => return Err(paper_setup_error(err)),
            }
        }
        if waited >= 600 {
            return Err(RpcError::Network(format!(
                "Paperless has not answered at {base_url} after ten minutes; `docker compose logs` in its folder says why"
            )));
        }
        if waited.is_multiple_of(15) {
            progress(format!(
                "waiting for Paperless to finish starting ({waited} s)…"
            ));
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
        waited += 5;
    }
}

/// Whether a postal address's Paperless is up, for the sidebar to say.
#[derive(Debug, Clone, Serialize)]
pub struct PaperlessHealth {
    pub id: i64,
    pub base_url: String,
    pub answering: bool,
    /// It is the Paperless kuverta installed, so kuverta can start it. One
    /// that runs elsewhere is somebody else's to start.
    pub startable: bool,
}

/// Asks each address's Paperless whether it is there. `mailboxes` is
/// `(id, base_url)`.
pub async fn paperless_health(
    mailboxes: &[(i64, String)],
    data_dir: &Path,
) -> Vec<PaperlessHealth> {
    let ours = installed_paperless(data_dir).map(|(url, _)| normal_url(&url));
    let mut health = Vec::with_capacity(mailboxes.len());
    for (id, base_url) in mailboxes {
        health.push(PaperlessHealth {
            id: *id,
            base_url: base_url.clone(),
            answering: core_paper::probe(base_url).await == core_paper::Probe::Paperless,
            startable: ours.as_deref() == Some(normal_url(base_url).as_str()),
        });
    }
    health
}

fn normal_url(url: &str) -> String {
    url.trim()
        .trim_end_matches('/')
        .to_ascii_lowercase()
        .replace("://127.0.0.1", "://localhost")
}

/// Starts the Paperless kuverta installed, and waits until it answers:
/// Docker first when its engine is not running — after a restart it usually
/// is not — then Paperless's containers. Returns its address.
pub async fn start_paperless(
    data_dir: &Path,
    progress: impl Fn(String) + Send + Sync + Clone + 'static,
) -> Result<String> {
    let (base_url, dir) = installed_paperless(data_dir).ok_or_else(|| {
        RpcError::Rejected("kuverta did not install this Paperless, so it cannot start it".into())
    })?;

    let docker = tokio::task::spawn_blocking(docker_status)
        .await
        .map_err(|err| RpcError::Rejected(err.to_string()))?;
    if !docker.installed {
        return Err(RpcError::Rejected(
            "Docker is not installed, and Paperless runs in it".into(),
        ));
    }
    if !docker.running {
        progress("starting Docker…".into());
        start_docker()?;
        let mut waited = 0u64;
        loop {
            tokio::time::sleep(Duration::from_secs(3)).await;
            waited += 3;
            let running = tokio::task::spawn_blocking(docker_status)
                .await
                .map_err(|err| RpcError::Rejected(err.to_string()))?
                .running;
            if running {
                break;
            }
            if waited >= 180 {
                return Err(RpcError::Rejected(
                    "Docker has not started after three minutes; open it and look at what it says"
                        .into(),
                ));
            }
            if waited.is_multiple_of(15) {
                progress(format!("waiting for Docker to start ({waited} s)…"));
            }
        }
    }

    progress("starting Paperless…".into());
    let output = progress.clone();
    tokio::task::spawn_blocking(move || compose_up(&dir, output))
        .await
        .map_err(|err| RpcError::Rejected(err.to_string()))??;

    let mut waited = 0u64;
    while core_paper::probe(&base_url).await != core_paper::Probe::Paperless {
        if waited >= 300 {
            return Err(RpcError::Network(format!(
                "Paperless has not answered at {base_url} after five minutes; `docker compose logs` in {} says why",
                data_dir.join("paperless").display()
            )));
        }
        if waited.is_multiple_of(15) {
            progress(format!("waiting for Paperless to answer ({waited} s)…"));
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
        waited += 3;
    }
    progress("Paperless is running.".into());
    Ok(base_url)
}

/// A Paperless user's token, from their name and password.
pub async fn sign_in_paperless(base_url: &str, username: &str, password: &str) -> Result<String> {
    if core_paper::probe(base_url).await != core_paper::Probe::Paperless {
        return Err(RpcError::Network(format!(
            "no Paperless answers at {}",
            base_url.trim()
        )));
    }
    core_paper::obtain_token(base_url, username, password)
        .await
        .map_err(paper_setup_error)
}

/// A Paperless sign-in's failure, in words that say what to do.
pub fn paper_setup_error(err: core_paper::PaperError) -> RpcError {
    use core_paper::PaperError;
    match err {
        PaperError::Auth => RpcError::Auth("Paperless did not accept that user name and password".into()),
        PaperError::Status { status: 429, .. } => RpcError::Auth(
            "Paperless refuses more sign-ins for now after several attempts; wait a minute and try again".into(),
        ),
        PaperError::Network(detail) => RpcError::Network(format!("Paperless is not answering ({detail})")),
        other => RpcError::Rejected(other.to_string()),
    }
}

/// Checks an address and token before they are saved, so a mistake is
/// reported in the assistant rather than as an empty postbox later.
pub async fn check_paperless(
    base_url: &str,
    token: &str,
    selector_kind: &str,
    selector_value: Option<&str>,
) -> Result<core_paper::PaperReport> {
    let selector = crate::paper::selector_from(selector_kind, selector_value)?;
    let client = core_paper::Paperless::new(base_url.trim(), token)
        .map_err(|err| RpcError::Rejected(err.to_string()))?;
    client.check(&selector).await.map_err(|err| match err {
        core_paper::PaperError::Auth => {
            RpcError::Auth("Paperless did not accept that token".into())
        }
        other => paper_setup_error(other),
    })
}

// -- mail accounts other programs know -----------------------------------------------

/// Where a program's password help sends someone.
#[derive(Debug, Clone, Serialize)]
pub struct PasswordHelp {
    pub text: String,
    pub url: String,
}

/// An account found on this computer or for a typed-in address, ready to
/// save once it has a password.
#[derive(Debug, Clone, Serialize)]
pub struct FoundAccount {
    /// `Thunderbird`, `Apple Mail`, or where the settings for a typed-in
    /// address came from.
    pub source: String,
    pub input: AccountInput,
    pub full_name: Option<String>,
    pub already_added: bool,
    /// Whether the servers are known. An Apple Mail account whose provider
    /// nobody could name has only its address.
    pub complete: bool,
    /// Whether the program it came from has the password too, so that nobody
    /// has to type it again. The password itself stays in the core.
    pub password_known: bool,
    pub password_help: Option<PasswordHelp>,
    pub notes: Vec<String>,
}

/// How looking in one program went.
#[derive(Debug, Clone, Serialize)]
pub struct ImportSource {
    pub name: String,
    /// `found`, `empty`, `absent`, `not_permitted` or `failed`.
    pub state: String,
    pub detail: String,
    /// A page that fixes it, for `not_permitted`.
    pub action_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportScan {
    pub accounts: Vec<FoundAccount>,
    pub sources: Vec<ImportSource>,
    /// Thunderbird has saved passwords but keeps them behind its primary
    /// password, which the assistant asks for and passes back.
    pub passwords_locked: bool,
}

fn help_for(input: &AccountInput) -> Option<PasswordHelp> {
    autoconfig::password_help(&input.imap_host).map(|(text, url)| PasswordHelp {
        text: text.to_string(),
        url: url.to_string(),
    })
}

fn found(source: &str, input: AccountInput, existing: &[String]) -> FoundAccount {
    let mut notes = Vec::new();
    if input.auth_method == "oauth2" {
        notes.push(
            "Signs in with OAuth2, which needs a client id and `kuverta login` in a terminal. \
             Switch it to an app password if the provider offers one."
                .to_string(),
        );
    }
    FoundAccount {
        already_added: existing
            .iter()
            .any(|email| email.eq_ignore_ascii_case(&input.email)),
        password_help: help_for(&input),
        complete: !input.imap_host.is_empty(),
        password_known: false,
        source: source.to_string(),
        full_name: None,
        input,
        notes,
    }
}

/// A Thunderbird login that matches an account, by its IMAP server and user
/// name — what Thunderbird itself signs in with.
fn login_for<'a>(
    logins: &'a [passwords::Login],
    input: &AccountInput,
) -> Option<&'a passwords::Login> {
    let host = input.imap_host.trim().to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let wanted = format!("imap://{host}");
    let user = input
        .username
        .clone()
        .unwrap_or_else(|| input.email.clone())
        .to_ascii_lowercase();
    let local = input
        .email
        .split('@')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    logins.iter().find(|login| {
        login.origin.eq_ignore_ascii_case(&wanted) && {
            let name = login.username.to_ascii_lowercase();
            name == user || name == local || name == input.email.to_ascii_lowercase()
        }
    })
}

/// The passwords Thunderbird has saved, across its profiles, and whether any
/// profile keeps them behind a primary password.
fn thunderbird_logins(primary_password: &str) -> (Vec<passwords::Login>, bool) {
    let mut found = Vec::new();
    let mut locked = false;
    for root in thunderbird::roots() {
        for profile in thunderbird::profiles(&root).unwrap_or_default() {
            match passwords::read(&profile, primary_password) {
                Ok(logins) => found.extend(logins),
                Err(passwords::PasswordError::NeedsPrimaryPassword)
                | Err(passwords::PasswordError::WrongPrimaryPassword) => locked = true,
                Err(passwords::PasswordError::None) => {}
                Err(err) => tracing::warn!(%err, "could not read Thunderbird's passwords"),
            }
        }
    }
    (found, locked)
}

/// The password another program has for an account, for the keychain. Never
/// returned to a window.
pub fn stored_password(input: &AccountInput, primary_password: &str) -> Option<String> {
    let (logins, _) = thunderbird_logins(primary_password);
    login_for(&logins, input).map(|login| login.password.clone())
}

/// Every mail account Thunderbird and Apple Mail know on this computer.
/// `existing` are the addresses kuverta already has.
pub async fn scan_imports(existing: &[String], primary_password: &str) -> ImportScan {
    let mut accounts: Vec<FoundAccount> = Vec::new();
    let mut sources = Vec::new();
    let (logins, passwords_locked) = thunderbird_logins(primary_password);
    let add = |account: FoundAccount, accounts: &mut Vec<FoundAccount>| {
        let seen = accounts
            .iter()
            .any(|known| known.input.email.eq_ignore_ascii_case(&account.input.email));
        if !seen {
            accounts.push(account);
        }
    };

    match thunderbird::scan() {
        Ok(None) => sources.push(ImportSource {
            name: "Thunderbird".into(),
            state: "absent".into(),
            detail: "not used on this computer".into(),
            action_url: None,
        }),
        Ok(Some(list)) => {
            sources.push(ImportSource {
                name: "Thunderbird".into(),
                state: if list.is_empty() { "empty" } else { "found" }.into(),
                detail: match list.len() {
                    0 => "no IMAP accounts".to_string(),
                    1 => "1 account".to_string(),
                    n => format!("{n} accounts"),
                },
                action_url: None,
            });
            for account in list {
                let mut entry = found("Thunderbird", account.input, existing);
                entry.password_known = login_for(&logins, &entry.input).is_some();
                entry.full_name = account.full_name;
                if account.was_cleartext {
                    entry.notes.push(
                        "Thunderbird connects to this server without encryption; kuverta will use STARTTLS."
                            .into(),
                    );
                }
                add(entry, &mut accounts);
            }
        }
        Err(err) => sources.push(ImportSource {
            name: "Thunderbird".into(),
            state: if err.kind() == std::io::ErrorKind::PermissionDenied {
                "not_permitted"
            } else {
                "failed"
            }
            .into(),
            detail: err.to_string(),
            action_url: None,
        }),
    }

    match apple_mail::scan() {
        apple_mail::Outcome::Absent => {
            if platform() == "macos" {
                sources.push(ImportSource {
                    name: "Apple Mail".into(),
                    state: "absent".into(),
                    detail: "no accounts set up".into(),
                    action_url: None,
                });
            }
        }
        apple_mail::Outcome::NotPermitted => sources.push(ImportSource {
            name: "Apple Mail".into(),
            state: "not_permitted".into(),
            detail: "macOS keeps Mail's accounts private until kuverta has Full Disk Access. \
                     Turn it on for kuverta (or the terminal it runs from), then look again."
                .into(),
            action_url: Some(apple_mail::PERMISSION_SETTINGS.into()),
        }),
        apple_mail::Outcome::Found(list) => {
            sources.push(ImportSource {
                name: "Apple Mail".into(),
                state: if list.is_empty() { "empty" } else { "found" }.into(),
                detail: match list.len() {
                    0 => "no mail accounts".to_string(),
                    1 => "1 account".to_string(),
                    n => format!("{n} accounts"),
                },
                action_url: None,
            });
            for account in list {
                let entry = apple_account(account, existing).await;
                add(entry, &mut accounts);
            }
        }
    }

    ImportScan {
        accounts,
        sources,
        passwords_locked,
    }
}

async fn apple_account(account: apple_mail::AppleAccount, existing: &[String]) -> FoundAccount {
    let discovered = autoconfig::discover(&account.email).await;
    let mut input = match (&account.imap, &discovered) {
        (Some((host, port, ssl)), _) => {
            let security = if *ssl == Some(false) {
                "starttls"
            } else {
                "tls"
            };
            let mut input = AccountInput {
                label: account.email.clone(),
                email: account.email.clone(),
                imap_host: host.clone(),
                imap_port: port.unwrap_or(if security == "tls" { 993 } else { 143 }),
                imap_security: security.to_string(),
                auth_method: "app_password".to_string(),
                ..AccountInput::default()
            };
            // The outgoing server is a separate account in macOS; the
            // provider's published one stands in for it.
            if let Some(found) = &discovered {
                input.smtp_host = found.input.smtp_host.clone();
                input.smtp_port = found.input.smtp_port;
                input.smtp_security = found.input.smtp_security.clone();
            }
            input
        }
        (None, Some(found)) => found.input.clone(),
        (None, None) => AccountInput {
            label: account.email.clone(),
            email: account.email.clone(),
            imap_port: 993,
            imap_security: "tls".to_string(),
            auth_method: "app_password".to_string(),
            ..AccountInput::default()
        },
    };
    if let Some(description) = &account.description {
        input.label = description.clone();
    }
    if input.username.is_none() {
        input.username = account.username.clone();
    }
    let mut entry = found("Apple Mail", input, existing);
    if !entry.complete {
        entry
            .notes
            .push("No server settings were found for this address; enter them.".into());
    } else if entry.input.smtp_host.is_none() {
        entry
            .notes
            .push("No outgoing server was found; add one to send from this address.".into());
    }
    entry
}

/// Settings for an address typed into the assistant.
pub async fn lookup(email: &str, existing: &[String]) -> Result<FoundAccount> {
    if !email.contains('@') {
        return Err(RpcError::Rejected(format!(
            "{email} is not an email address"
        )));
    }
    let entry = match autoconfig::discover(email).await {
        Some(discovered) => found(discovered.source, discovered.input, existing),
        None => {
            let mut entry = found(
                "a guess",
                AccountInput {
                    label: email.trim().to_string(),
                    email: email.trim().to_string(),
                    imap_host: String::new(),
                    imap_port: 993,
                    imap_security: "tls".to_string(),
                    auth_method: "app_password".to_string(),
                    ..AccountInput::default()
                },
                existing,
            );
            entry.notes.push(
                "Nobody publishes settings for this address. Your provider's help pages name the IMAP and SMTP servers."
                    .into(),
            );
            entry
        }
    };
    Ok(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_install_keeps_its_port_and_key_and_keeps_its_password_private() {
        let dir = std::env::temp_dir().join(format!("kuverta-setup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let install = PaperlessInstall {
            username: "erika".into(),
            password: "correct horse".into(),
            on_network: false,
        };
        let (url, folder) = write_paperless(&dir, &install).unwrap();
        assert!(url.starts_with("http://localhost:"));
        assert_eq!(
            installed_paperless(&dir),
            Some((url.clone(), folder.clone()))
        );

        let compose = std::fs::read_to_string(folder.join("docker-compose.yml")).unwrap();
        assert!(compose.contains(PAPERLESS_IMAGE));
        assert!(compose.contains("\"127.0.0.1:"), "{compose}");
        let env = std::fs::read_to_string(folder.join("paperless.env")).unwrap();
        assert!(env.contains("PAPERLESS_ADMIN_USER=erika\n"));
        assert!(env.contains("PAPERLESS_ADMIN_PASSWORD=correct horse\n"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(folder.join("paperless.env"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let key = |env: &str| {
            env.lines()
                .find_map(|line| line.strip_prefix("PAPERLESS_SECRET_KEY="))
                .unwrap()
                .to_string()
        };
        assert_eq!(key(&env).len(), 64);

        // Again, now on the network: same address, same key.
        let again = PaperlessInstall {
            on_network: true,
            ..install
        };
        let (url_again, _) = write_paperless(&dir, &again).unwrap();
        assert_eq!(url_again, url);
        let env_again = std::fs::read_to_string(folder.join("paperless.env")).unwrap();
        assert_eq!(key(&env_again), key(&env));
        let compose = std::fs::read_to_string(folder.join("docker-compose.yml")).unwrap();
        assert!(!compose.contains("127.0.0.1"), "{compose}");

        let short = PaperlessInstall {
            username: "erika".into(),
            password: "short".into(),
            on_network: false,
        };
        assert!(write_paperless(&dir, &short).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_install_that_exists_keeps_the_compose_project_it_was_written_with() {
        let dir = std::env::temp_dir().join(format!("kuverta-project-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paperless = paperless_dir(&dir);
        std::fs::create_dir_all(&paperless).unwrap();

        // Nothing there yet: the name is this instance's. The tests run
        // without KUVERTA_INSTANCE, so that is the plain one.
        assert_eq!(compose_project(&paperless), "kuverta-paperless");

        // Renaming an install would leave its volumes — its post — behind,
        // so whatever it says stays.
        std::fs::write(
            paperless.join("docker-compose.yml"),
            "name: kuverta-paperless-dev\nservices: {}\n",
        )
        .unwrap();
        assert_eq!(compose_project(&paperless), "kuverta-paperless-dev");

        let install = PaperlessInstall {
            username: "erika".into(),
            password: "a long enough password".into(),
            on_network: false,
        };
        let (_, folder) = write_paperless(&dir, &install).unwrap();
        let compose = std::fs::read_to_string(folder.join("docker-compose.yml")).unwrap();
        assert!(
            compose.contains("name: kuverta-paperless-dev\n"),
            "{compose}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn only_the_paperless_kuverta_installed_can_be_started_from_here() {
        let dir = std::env::temp_dir().join(format!("kuverta-setup-health-{}", std::process::id()));
        let install = PaperlessInstall {
            username: "erika".into(),
            password: "a long enough password".into(),
            on_network: false,
        };
        let (ours, _) = write_paperless(&dir, &install).unwrap();
        let theirs = "http://127.0.0.1:9".to_string();

        let health = paperless_health(&[(1, format!("{ours}/")), (2, theirs)], &dir).await;
        assert!(health[0].startable);
        assert!(!health[1].startable);
        // Nothing was started, so nothing answers.
        assert!(!health[0].answering && !health[1].answering);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn setup_is_finished_once_and_stays_finished() {
        let dir = std::env::temp_dir().join(format!("kuverta-setup-marker-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let core = Core::open(&dir).unwrap();
        let status = core.setup_status(&dir).unwrap();
        assert!(status.first_run && !status.finished);

        finish(&dir).unwrap();
        let status = core.setup_status(&dir).unwrap();
        assert!(!status.first_run && status.finished);
        // Closed first: Windows does not delete a database that is open.
        drop(core);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_saved_login_is_matched_to_its_account_by_server_and_user() {
        let login = |origin: &str, user: &str| passwords::Login {
            origin: origin.to_string(),
            username: user.to_string(),
            password: "secret".to_string(),
        };
        let logins = vec![
            login("imap://mail.example.de", "erika"),
            login("smtp://mail.example.de", "erika"),
            login("imap://imap.gmail.com", "erika.m@gmail.com"),
        ];
        let account = |email: &str, host: &str, user: Option<&str>| AccountInput {
            email: email.into(),
            imap_host: host.into(),
            username: user.map(str::to_string),
            ..AccountInput::default()
        };

        // The user name Thunderbird signs in with, the address, or its local
        // part — all three are seen in the wild.
        assert!(login_for(
            &logins,
            &account("erika@example.de", "mail.example.de", Some("erika"))
        )
        .is_some());
        assert!(login_for(
            &logins,
            &account("erika@example.de", "mail.example.de", None)
        )
        .is_some());
        assert!(login_for(
            &logins,
            &account("erika.m@gmail.com", "imap.gmail.com", None)
        )
        .is_some());
        // The incoming server only: a login for the outgoing one is not it.
        assert!(login_for(
            &logins,
            &account("erika@example.de", "smtp.example.de", None)
        )
        .is_none());
        assert!(login_for(
            &logins,
            &account("someone@example.de", "mail.example.de", None)
        )
        .is_none());
        assert!(login_for(&logins, &account("erika@example.de", "", None)).is_none());
    }

    #[test]
    fn found_accounts_know_whether_they_are_already_there() {
        let existing = vec!["Erika@example.de".to_string()];
        let input = autoconfig::known("erika.m@gmail.com").unwrap().input;
        let entry = found("Thunderbird", input, &existing);
        assert!(!entry.already_added);
        assert!(entry.complete);
        assert!(entry.password_help.unwrap().url.contains("apppasswords"));

        let input = AccountInput {
            email: "erika@example.de".into(),
            imap_host: "imap.example.de".into(),
            ..AccountInput::default()
        };
        assert!(found("Thunderbird", input, &existing).already_added);
    }
}
