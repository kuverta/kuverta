---
description: Which models kuverta uses, running them locally with Ollama or with a hosted OpenAI-compatible service, and what each choice sends off your computer.
---

# Models

kuverta has three jobs a language model can do:

| Job | Needs | Default |
| --- | --- | --- |
| **Reading scanned letters**: turning a photographed or scanned page into text | a model that can see images | `qwen2.5vl:3b` |
| **Sorting mail**: a second opinion on each message's category, used by `kuverta classify` | any chat model | `qwen3:4b-instruct` |
| **The assistant**: the [chat and its tasks](assistant.md) | a chat model that can call tools; larger is better | the sorting model, until one is chosen |

They run with [Ollama](https://ollama.com) **on your computer** unless you choose
otherwise. The [setup assistant](../install.md#the-first-run) checks for Ollama,
starts it if it is stopped, and downloads the models with progress.

!!! note "The model does not decide what the list shows"
    The categories in the list come from the rules classifier and your own
    corrections. A model's verdicts are recorded beside the rules' and compared
    with them (`kuverta disagreements` lists where they differ), but they do
    not change what you see. See the [command line](../developers/cli.md#models).

## Choosing a model for each job

**Settings → Models → Model for each job** has a **Provider** and a **Model** for
each job. The model field offers what the chosen provider has. The note under
it says:

- if a model is not on this Ollama yet, with the `ollama pull …` command to get it;
- if a model **cannot see images** and so cannot read scans, or if the
  provider does not say, in which case **Try them** finds out;
- for any provider that is not on this computer, **what the job would send
  there**.

![Settings → Models](../assets/screens/settings-models-light.webp#only-light)
![Settings → Models](../assets/screens/settings-models-dark.webp#only-dark)

**Try them** runs each job once on a built-in sample (never on your mail) and
reports whether it worked and how long it took. **Save** keeps the choice.

## How the sorting models were measured

The sorting default was chosen by measurement, and you can rerun it on any
model Ollama can pull. The test set is generated, so no real mail is involved
and every message has a known right answer.

```sh
python3 docker/fill-mailbox.py --dump 250 | node kuverta-bird/tools/dump-facts.js > labelled.jsonl
kuverta eval labelled.jsonl --model qwen3:4b-instruct --show-wrong
kuverta eval labelled.jsonl --model qwen3:4b-instruct --split sender
```

`eval` holds out half of the messages and scores the rules, the prompted model
and the nearest-filings classifier on the same half. `--split sender` holds out
whole senders, so the model is scored only on addresses it has never seen.
`--show-wrong` lists every message a method got wrong, and the choice was made
on that list.

Measured 21 November 2026 on an Apple M2 with 24 GB, Ollama 0.40, 250 generated
messages in German and English, 125 scored:

| model | licence | size | every other message | senders never seen | per message |
|---|---|---|---|---|---|
| **`qwen3:4b-instruct`** (default) | Apache-2.0 | 4.0B | 92.0% | 100% | ~360 ms |
| `phi4-mini` | MIT | 3.8B | 93.6% | 100% | ~270 ms |
| `granite3.3:2b` | Apache-2.0 | 2.5B | 76.0% | n/a | ~270 ms |
| `qwen3:4b` (thinking on) | Apache-2.0 | 4.0B | no usable answer | n/a | ~740 ms |
| `qwen2.5:3b` | Qwen research licence | 3.1B | 87.2% | 97.3% | ~285 ms |
| `gemma3:4b` | Gemma licence | 4.3B | 87.2% | 76.8% | ~850 ms |

Every miss of the two best models is one of two templates: a bank's "your
statement is ready" and the tax office's "your assessment is ready". They were
filed as notification where the test set says transactional, and a careful
person could file them either way. On senders they had never seen both were
perfect. The default is the Qwen model, although `phi4-mini` is slightly more
accurate, because the same default also runs the assistant, where tool calling
and German matter more than a tenth of a second. In the app, your own earlier
filings decide first when they agree on where a message goes, and that
combination scored 100% with either model.

Thinking models (`qwen3:4b` without `-instruct`, `deepseek-r1`, …) spend the
short answer budget on reasoning and return nothing usable for sorting. Pick a
non-thinking build.

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
never shown again. Each provider's page lists its models, their size, and
whether each can see images (where the provider says).

## What leaves your computer

Only a provider on **this computer** (Ollama at `localhost`) keeps everything
local. For any other, including an Ollama on another machine in your network:

| Job | Sent to the provider |
| --- | --- |
| Reading scanned letters | the **scans of your letters** |
| Sorting mail | the **sender, subject and start of each message** |
| The assistant | **whatever it searches for and opens**, and for tasks that ask a model, the message they are about |

The settings page states this under each job whenever it applies. A hosted
service needs no download and is often better and faster. Whether that is
worth sending your post or mail to it is your decision, job by job.
