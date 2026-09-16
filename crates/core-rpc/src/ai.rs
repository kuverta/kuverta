//! Where models run, and which model each job uses.
//!
//! The settings behind the models page, and the one place the rest of the app
//! asks "which model, where?". A job nobody has chosen for runs where it always
//! did: on the Ollama on this computer, with the model it was measured with.
//!
//! A hosted service is chosen, never fallen back to. Whatever a job sends goes
//! with it — a letter's scan, a message's sender and subject — so a job moves
//! off this computer only when someone points it elsewhere, and the page says
//! what that sends.

use serde::{Deserialize, Serialize};

use core_store::{NewAiProvider, StoredAiProvider};

use crate::{Core, Result, RpcError};

/// The provider every store starts with: Ollama, on this computer.
pub const LOCAL_PROVIDER: i64 = 1;

/// A page with known words on it, for trying whether a model can read one.
const TRY_PAGE: &[u8] = include_bytes!("../assets/try-page.jpg");
const TRY_PAGE_SAYS: &str = "4711";

/// A job kuverta gives a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Task {
    /// Reading the text off a photographed letter. Needs a model that can see.
    Vision,
    /// Sorting mail into categories, as `kuverta classify` does.
    Chat,
}

impl Task {
    pub const ALL: [Task; 2] = [Task::Vision, Task::Chat];

    pub fn as_str(self) -> &'static str {
        match self {
            Task::Vision => "vision",
            Task::Chat => "chat",
        }
    }

    /// The model a job uses on this computer until another is chosen.
    ///
    /// `KUVERTA_VISION_MODEL` still names the vision default, as it did before
    /// there was a page for it.
    pub fn default_model(self) -> String {
        match self {
            Task::Vision => std::env::var("KUVERTA_VISION_MODEL")
                .ok()
                .filter(|model| !model.trim().is_empty())
                .unwrap_or_else(|| "qwen2.5vl:3b".to_string()),
            Task::Chat => "llama3.2:3b".to_string(),
        }
    }
}

/// A provider, as the models page shows it. Never the key.
#[derive(Debug, Clone, Serialize)]
pub struct AiProviderView {
    pub id: i64,
    pub kind: String,
    pub label: String,
    pub base_url: String,
    pub has_key: bool,
    /// Whether it runs on this computer, which decides whether a job's data
    /// leaves it.
    pub local: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiProviderInput {
    /// Carried so that editing an address changes the provider rather than
    /// adding a second one and stranding the key filed for the first.
    pub id: Option<i64>,
    pub kind: String,
    pub label: String,
    pub base_url: String,
}

/// The model a job uses, chosen or by default.
#[derive(Debug, Clone, Serialize)]
pub struct AiTaskView {
    pub task: Task,
    pub provider_id: i64,
    pub model: String,
    pub chosen: bool,
}

/// What a job will use: a client ready to ask, built so no lock need be held
/// while it is asked.
pub struct AiChoice {
    pub provider: core_ai::Provider,
    pub model: String,
    pub provider_label: String,
    pub local: bool,
}

/// How trying a model at a job went.
#[derive(Debug, Clone, Serialize)]
pub struct AiTrial {
    pub reply: String,
    pub latency_ms: i64,
    pub passed: bool,
    pub verdict: String,
}

/// Where a hosted service's key is filed in the keychain.
fn key_entry(provider: &StoredAiProvider) -> core_accounts::KeychainPassword {
    core_accounts::KeychainPassword::new(format!("ai:{}", provider.key_name))
}

/// Whether an address is this computer. An Ollama elsewhere on the network is
/// not, and neither is a hostname, which could resolve anywhere.
pub fn is_loopback(base_url: &str) -> bool {
    let rest = base_url
        .split_once("://")
        .map_or(base_url, |(_, rest)| rest);
    let authority = rest.split('/').next().unwrap_or_default();
    let host = match authority.strip_prefix('[') {
        Some(bracketed) => bracketed.split(']').next().unwrap_or_default(),
        None => authority
            .rsplit_once(':')
            .map_or(authority, |(host, _)| host),
    };
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

impl Core {
    pub fn ai_providers(&self) -> Result<Vec<AiProviderView>> {
        Ok(self.store.ai_providers()?.iter().map(view).collect())
    }

    pub fn save_ai_provider(&self, input: &AiProviderInput) -> Result<i64> {
        let label = input.label.trim();
        let base_url = input.base_url.trim();
        // Checked by building a client, which is the check that matters.
        core_ai::Provider::connect(&input.kind, base_url, None)
            .map_err(|err| RpcError::Rejected(err.to_string()))?;
        if label.is_empty() {
            return Err(RpcError::Rejected("a provider needs a name".into()));
        }

        let provider = NewAiProvider {
            kind: input.kind.clone(),
            label: label.to_string(),
            base_url: base_url.to_string(),
        };
        match input.id {
            Some(id) => {
                if !self.store.update_ai_provider(id, &provider)? {
                    return Err(RpcError::Rejected(format!("no model provider {id}")));
                }
                Ok(id)
            }
            None => Ok(self.store.add_ai_provider(&provider)?),
        }
    }

    pub fn delete_ai_provider(&self, id: i64) -> Result<()> {
        if id == LOCAL_PROVIDER {
            return Err(RpcError::Rejected(
                "the Ollama on this computer stays; point it at another address instead".into(),
            ));
        }
        if let Some(provider) = self.store.ai_provider(id)? {
            // As with Paperless tokens: no key is left behind for a provider
            // nobody can see any more.
            let _ = key_entry(&provider).delete();
        }
        Ok(self.store.delete_ai_provider(id)?)
    }

    /// Stores a hosted service's key in the keychain. Nothing reads it back
    /// out but the client that sends it to that service.
    pub fn set_ai_key(&self, id: i64, key: &str) -> Result<()> {
        let provider = self.stored_ai_provider(id)?;
        let key = key.trim();
        if key.is_empty() {
            return Err(RpcError::Rejected("an empty key is not a key".into()));
        }
        key_entry(&provider)
            .store(key)
            .map_err(|err| RpcError::Auth(err.to_string()))
    }

    /// Every job, with the model it uses now.
    pub fn ai_tasks(&self) -> Result<Vec<AiTaskView>> {
        let chosen = self.store.ai_tasks()?;
        Ok(Task::ALL
            .iter()
            .map(
                |&task| match chosen.iter().find(|row| row.task == task.as_str()) {
                    Some(row) => AiTaskView {
                        task,
                        provider_id: row.provider_id,
                        model: row.model.clone(),
                        chosen: true,
                    },
                    None => AiTaskView {
                        task,
                        provider_id: LOCAL_PROVIDER,
                        model: task.default_model(),
                        chosen: false,
                    },
                },
            )
            .collect())
    }

    pub fn set_ai_task(&self, task: Task, provider_id: i64, model: &str) -> Result<()> {
        self.stored_ai_provider(provider_id)?;
        let model = model.trim();
        if model.is_empty() {
            return Err(RpcError::Rejected(format!(
                "choose a model for {}",
                task.as_str()
            )));
        }
        Ok(self.store.set_ai_task(task.as_str(), provider_id, model)?)
    }

    /// Sends a job back to its default.
    pub fn reset_ai_task(&self, task: Task) -> Result<()> {
        Ok(self.store.clear_ai_task(task.as_str())?)
    }

    /// A client for one provider, with its key if it has one.
    pub fn ai_provider_client(&self, id: i64) -> Result<core_ai::Provider> {
        connect(&self.stored_ai_provider(id)?)
    }

    /// The provider and model a job uses now.
    pub fn ai_for(&self, task: Task) -> Result<AiChoice> {
        let chosen = self
            .ai_tasks()?
            .into_iter()
            .find(|view| view.task == task)
            .ok_or_else(|| RpcError::Rejected(format!("no job {}", task.as_str())))?;
        let provider = self.stored_ai_provider(chosen.provider_id)?;
        Ok(AiChoice {
            provider: connect(&provider)?,
            model: chosen.model,
            local: is_loopback(&provider.base_url),
            provider_label: provider.label,
        })
    }

    fn stored_ai_provider(&self, id: i64) -> Result<StoredAiProvider> {
        self.store
            .ai_provider(id)?
            .ok_or_else(|| RpcError::Rejected(format!("no model provider {id}")))
    }
}

fn view(provider: &StoredAiProvider) -> AiProviderView {
    AiProviderView {
        id: provider.id,
        kind: provider.kind.clone(),
        label: provider.label.clone(),
        base_url: provider.base_url.clone(),
        // Only a hosted service has a key to look for; asking the keychain
        // about an Ollama would be a keychain read for nothing.
        has_key: provider.kind == "openai" && matches!(key_entry(provider).peek(), Ok(Some(_))),
        local: is_loopback(&provider.base_url),
    }
}

fn connect(provider: &StoredAiProvider) -> Result<core_ai::Provider> {
    let key = if provider.kind == "openai" {
        key_entry(provider)
            .peek()
            .map_err(|err| RpcError::Auth(err.to_string()))?
    } else {
        None
    };
    core_ai::Provider::connect(&provider.kind, &provider.base_url, key)
        .map_err(|err| RpcError::Rejected(err.to_string()))
}

/// What a provider offers. Asked with no lock held.
pub async fn list_models(provider: &core_ai::Provider) -> Result<Vec<core_ai::ModelInfo>> {
    provider
        .models()
        .await
        .map_err(|err| ai_error(provider.base_url(), err))
}

/// After a page read a second time, because the first reading looped.
pub const READ_AGAIN_NOTE: &str = "[the model looped on this page and read it a second time, \
more loosely: check names and numbers against the scan]";

/// After a page the model ran out of room on.
pub const STOPPED_NOTE: &str = "[the model stopped before the end of this page]";

/// One page's text, as a vision model reads it.
///
/// A plain reading first, because it is the most exact. If it loops or runs
/// out of room, a second reading that penalises repetition — which gets
/// through tables the plain one loops on, and gets numbers wrong more often —
/// kept only if it finishes cleanly, and marked as what it is. Failing that,
/// the first reading, with any loop cut out and marked.
pub async fn read_page(provider: &core_ai::Provider, model: &str, jpeg: &[u8]) -> Result<String> {
    let base = provider.base_url();
    let first = provider
        .transcribe(model, jpeg)
        .await
        .map_err(|err| ai_error(base, err))?;
    let (text, looped) = core_ai::cut_repetition(first.content.trim());
    if !looped && !first.truncated {
        return Ok(text);
    }

    if let Some(again) = provider
        .transcribe_guarded(model, jpeg)
        .await
        .map_err(|err| ai_error(base, err))?
    {
        let (again_text, again_looped) = core_ai::cut_repetition(again.content.trim());
        if !again_looped && !again.truncated {
            return Ok(format!("{again_text}\n{READ_AGAIN_NOTE}"));
        }
    }

    Ok(if looped {
        text
    } else {
        format!("{text}\n{STOPPED_NOTE}")
    })
}

/// Tries a model at a job: a page with known words on it for reading, a word
/// for sorting.
pub async fn try_model(provider: &core_ai::Provider, task: Task, model: &str) -> Result<AiTrial> {
    let base = provider.base_url();
    match task {
        Task::Vision => {
            let reply = provider
                .transcribe(model, TRY_PAGE)
                .await
                .map_err(|err| ai_error(base, err))?;
            let passed = reply.content.contains(TRY_PAGE_SAYS);
            Ok(AiTrial {
                verdict: if passed {
                    "read the sample page".into()
                } else {
                    format!("did not read the sample page, which says \"Rechnung {TRY_PAGE_SAYS}\"")
                },
                passed,
                reply: short(&reply.content),
                latency_ms: reply.latency_ms,
            })
        }
        Task::Chat => {
            let reply = provider
                .chat(model, "Reply with the single word: ok", "Are you there?")
                .await
                .map_err(|err| ai_error(base, err))?;
            let passed = !reply.content.trim().is_empty();
            Ok(AiTrial {
                verdict: if passed {
                    "answered".into()
                } else {
                    "answered with nothing; a model that reasons before answering may need more \
                     room than sorting gives it"
                        .into()
                },
                passed,
                reply: short(&reply.content),
                latency_ms: reply.latency_ms,
            })
        }
    }
}

fn short(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > 200 {
        format!("{}…", text.chars().take(200).collect::<String>())
    } else {
        text
    }
}

/// A model server's failure, in words that say what to do about it.
pub(crate) fn ai_error(base_url: &str, err: core_ai::AiError) -> RpcError {
    use core_ai::AiError;
    match err {
        AiError::Network(detail) if is_loopback(base_url) => RpcError::Network(format!(
            "{base_url} is not answering ({detail}); is Ollama running?"
        )),
        AiError::Network(detail) => {
            RpcError::Network(format!("{base_url} is not answering ({detail})"))
        }
        AiError::Status {
            status: 401 | 403,
            body,
        } => RpcError::Auth(format!("{base_url} refused the key: {body}")),
        AiError::Status { status: 404, body } => RpcError::Rejected(format!(
            "{base_url} answered 404, for a model it does not have or an address missing its /v1: {body}"
        )),
        other => RpcError::Rejected(other.to_string()),
    }
}
