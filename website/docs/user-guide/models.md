---
description: Which models kuverta uses, running them locally with Ollama or with a hosted OpenAI-compatible service, and what each choice sends off your computer.
---

# Models

kuverta has two jobs a language model can do:

| Job | Needs | Default |
| --- | --- | --- |
| **Reading scanned letters** — turning a photographed or scanned page into text | a model that can see images | `qwen2.5vl:3b` |
| **Sorting mail** — a second opinion on each message's category, used by `kuverta classify` | any chat model | `llama3.2:3b` |

Both run with [Ollama](https://ollama.com) **on your computer** unless you choose
otherwise. The [setup assistant](../install.md#the-first-run) checks for Ollama,
starts it if it is stopped, and downloads the models with progress.

!!! note "The model does not decide what the list shows"
    The categories in the list come from the rules classifier and your own
    corrections. A model's verdicts are recorded beside the rules' and compared
    with them — `kuverta disagreements` lists where they differ — but they do not
    change what you see. A model is kept honest by never being trusted on its own
    say-so. See the [command line](../developers/cli.md#models).

## Choosing a model for each job

**Settings → Models → Model for each job** has a **Provider** and a **Model** for
each job. The model field offers what the chosen provider has, and the note
under it says what matters:

- that a model is not on this Ollama yet, and the `ollama pull …` command to get it;
- that a model **cannot see images**, so it cannot read scans — or that the
  provider does not say, in which case **Try them** finds out;
- and, for any provider that is not on this computer, **what the job would send
  there**.

![Settings → Models](../assets/screens/settings-models-light.webp#only-light)
![Settings → Models](../assets/screens/settings-models-dark.webp#only-dark)

**Try them** runs each job once on a built-in sample — not on your mail — and
reports whether it worked and how long it took. **Save** keeps the choice.

## Providers

Press **+ Add** beside *Models* in settings to add a provider:

| Service | Address filled in |
| --- | --- |
| DeepSeek | `https://api.deepseek.com` |
| OpenAI | `https://api.openai.com/v1` |
| OpenRouter | `https://openrouter.ai/api/v1` |
| Another OpenAI-compatible service | the address you give |
| Ollama on another computer | the address you give |

A hosted service needs an **API key**, which is kept in the system keychain and
never shown again. Each provider's page lists the models it offers, whether each
can see images (where the provider says), and their size.

## What leaves your computer

Only a provider on **this computer** — Ollama at `localhost` — keeps everything
local. For any other, including an Ollama on another machine in your network:

| Job | Sent to the provider |
| --- | --- |
| Reading scanned letters | the **scans of your letters** |
| Sorting mail | the **sender, subject and start of each message** |

The settings page says this in so many words under each job whenever it
applies. A hosted service needs no download and is often better and faster;
whether that is worth sending your post or mail to it is your decision, job by
job.
