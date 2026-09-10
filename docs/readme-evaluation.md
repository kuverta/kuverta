# Evaluation of the original readme

Written 2026-09-10, before the decision review. Superseded operationally by
[`implementation-plan.md`](implementation-plan.md), kept for the reasoning.

## The core finding

The readme describes **two products**, sharing only the word "inbox":

- **A. Mail client** — Rust, IMAP/SMTP, Apple-Mail-like. Hard part: MIME/IMAP correctness, HTML rendering security, UI polish. Realistically 3–5 person-years to be trustworthy.
- **B. Paper-mail manager** — Pi + camera, OCR, auto-filing to cloud. Hard part: capture quality, OCR, classification. Medium difficulty, and largely solved by existing software.

Building them together is the main way this fails.

## Point by point

**"Rust? (feel free to suggest alternatives)"** — right for the engine (MIME/IMAP parsing is exactly the memory-unsafe attack surface that has burned every C mail client), wrong for a UI that must match Apple Mail. Hence Rust core + Tauri v2 shell.

**"Stupidly easy imap/smtp setup"** — a real differentiator, but it is a discovery cascade (RFC 6186 SRV → `autoconfig.<domain>` → `.well-known/autoconfig` → Mozilla ISPDB → Autodiscover → port probing), not a nice wizard. The hidden blocker was Google's restricted-scope verification plus annual CASA assessment for any *distributed* OAuth client — a recurring four-figure cost. Sidestepped for now by staying personal-use.

**"Extremely secure"** — not a feature, a set of constraints, and retrofitting them is what kills mail clients: no remote content by default, sandboxed HTML with strict CSP and no JS, credentials in the OS keychain, rustls, encrypted cache, fuzzed parsers. *Read-only v1 removes most of this surface, since nothing renders HTML and nothing can destroy mail.*

**"Looks like Outlook/Thunderbird/Apple Mail"** — the quality is in things that look free and are not: JWZ threading, instant search over 100k+ messages, virtualized scrolling, correct rendering of two decades of malformed HTML, offline sync that never loses a draft. The UI shell is ~20% of that work.

**"Base to plug into AI agents later"** — good instinct, cheap if done now: define the core as a library with a stable RPC surface, and an MCP server over it is a few hundred lines later. Do not build agent features in v1; just do not foreclose them.

**"Should run on pi/pizero"** — a Pi Zero cannot do OCR at tolerable speed and is marginal for dewarping. Split it: Pi captures, upstream does OCR. Also worth saying: for volume paper an ADF scanner beats a camera rig on quality; the rig wins on "drop it in the tray and walk away."

**"Simple heuristics and connected AI (lightllm)"** — read as **LiteLLM** (the router); `lightllm` is an unrelated inference server. Heuristics get 70–80% for free and generate the labelled data that makes AI worth adding. *Overridden by decision: local model from the start, with a rules baseline logged alongside.*

**"Base for SaaS with premium features"** — constrains architecture today: local-first with an optional server, never client-plus-mandatory-backend, or the free client cannot exist without your infrastructure.

## The central tension

"Extremely secure" and "connected AI reads all your mail" pull against each other. Resolve it in the product, not implicitly in code: local model by default, explicit opt-in for cloud, visible redaction, headers-plus-snippet rather than full bodies. Decided late, it is a rewrite. *(Resolved: local-only.)*

GDPR is not engaged while this is your own mail in a personal tool — the household exemption covers it. It becomes a real compliance question the moment you distribute or host.

## What the readme did not mention

Multi-account and unified inbox model · offline and conflict behaviour · calendar and contacts · notifications and push · import from existing clients · accessibility · i18n · update mechanism and signing · crash/telemetry policy · licensing · capacity.

Most of these are deferred by the read-only triage scope. Capacity turned out to be the decisive one.
