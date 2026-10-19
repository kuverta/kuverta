---
description: Needs attention ranks your Inbox by how soon each message needs you — worked out by kuverta's urgency agent from facts it checks and, if you like, your model's judgement.
---

# Needs attention

**Needs attention**, at the top of the sidebar, ranks the mail in your Inbox by
how soon it needs you. The number beside it is how many need you this week or
sooner. Each row says why, in one sentence, with what to do and by when.

This is a ranking of assessed Inbox mail, **not a guarantee that all important
or unread mail is listed**. New candidates normally cover the last 21 days;
older saved ratings may remain. A backlog across folders or older unread mail
needs a separate review in the assistant.

![Needs attention: three messages ranked, each with its reason](../assets/screens/attention-light.webp#only-light)
![Needs attention: three messages ranked, each with its reason](../assets/screens/attention-dark.webp#only-dark)

| Shown | Means |
| --- | --- |
| **today** | Act today or tomorrow: a deadline is close, a bill is overdue, someone is blocked. |
| **this week** | Act in the next few days. |
| *can wait* and *nothing to do* | Not listed here. |

After the level comes the action — *reply*, *pay*, *attend*, *decide* or *read* —
and a deadline when the message names one. Open a message and the same verdict
is shown under its subject, with who judged it.

## How it is worked out

A small agent reads the mail of the last three weeks that is in your Inbox and
is not bulk mail — newsletters, marketing and notifications are normally left to
[Cleanup](cleanup.md). A personal topic matching the sender or subject also
includes these categories. For each message it first checks what kuverta can find out
for itself:

- whether you have written to this sender before, and how often;
- whether you have **already answered** it — then it needs nothing;
- how many messages this sender has sent you before;
- whether it was written to you directly or to a group or a list;
- whether it talks about deadlines or asks for payment.

From those facts alone it forms a first verdict, at once, for every message.

Then, if you let it, it asks the model chosen for **sorting mail**
([Models](models.md)) about each message in turn. The model is given the facts
above and the start of the message, and answers with how soon, what to do, by
when and why. Its answer replaces the rules' verdict for that message. Two
things keep it honest:

- **A named deadline decides the level.** A small model calls a bill due in two
  weeks "today" as readily as one due tomorrow; a date can be counted. Overdue or
  within a day is *today*, within a week *this week*, later *can wait*.
- **The message is data, not instructions.** Mail is written by whoever sent it,
  and "URGENT — reply now" is advertising as often as not. The model is told that
  a sender's own claim of urgency is weak evidence and to lean on the facts, and
  the message is fenced so that nothing in it can pose as an instruction.

Each message is judged once; the verdicts are kept. The agent runs by itself
after every sync, quietly, and **Ask the model** in the list header runs it now.

## Choose your criteria

Under **Settings → General → Needs attention**, choose an account. The existing
deadline and payment words are editable suggestions, with one term per line.
**Personal topics to review** match the sender or subject, without regard to
case, and nominate matching recent Inbox mail for review. They do not prove that
a sender is legitimate or that a message needs action today. The model receives
these hints too and may decide that a message needs no action, including phishing.
Answered messages still need no action.

**Use original suggestions** fills the form; **Save attention criteria** applies
it. Saving changed criteria clears this account's stored ratings and recalculates
the local rules. If model assessment is enabled, subsequent syncs recheck with
the model (up to 150 candidates per pass). Saving itself makes no model request.
The criteria do not change other accounts or create mail-moving rules.

Ask the assistant to suggest additional topics. Its suggestion appears as a card
with the account, terms and explanation. Nothing changes until you click
**Apply suggestion**; you can edit or remove applied terms in settings.

## Review a large unread backlog

For a question such as “Which of my 1,000 unread mails matter?”, the assistant
is instructed to first use `scan_mail_headers`: up to 200 senders and subjects
per page, with dates, categories and message IDs. The result includes total,
returned count and the next offset. It contains no body, snippet or attachment
contents and does not fetch message blobs. This tool has no age cutoff and works
on the current account's locally cached mail. Trash/Junk are excluded from the
ordinary list unless explicitly requested as a folder.

The assistant then reads selected potentially important or unclear messages and
reports how many headers and bodies it actually reviewed, plus any remainder.
That sequence is model-guided: it is not a guarantee of complete coverage or
perfect importance detection. Messages arriving during pagination can change the
list. Merely asking for an assessment does not authorize moving or marking mail.

Example: “Review all unread mail on this account. First look only at senders and
subjects, then read potentially important or unclear messages. Show important
mail with reasons and links, separate uncertain cases, and name possible cleanup
candidates. Report your coverage. Do not change anything yet.”

With a hosted assistant, even senders and subjects leave the computer. Existing
chat history and messages explicitly attached to a question remain part of its
context. Start a new conversation without attached messages for a clean review.

## What leaves your computer

The rules run on your computer. The model runs wherever you chose it to: with
the default, Ollama on this computer, nothing leaves it. With a hosted service,
the start of each new message that is not bulk mail is sent to it, with the
checked facts. To keep the rules only, untick **After each sync, let the model
for sorting mail judge how soon new mail needs me** in **Settings → General**.

## For an assistant, and on the command line

The [MCP server](../developers/mcp.md) offers the same ranking as the
`urgent_messages` tool, so an assistant asked "what should I deal with first?"
answers from it. On the command line, `kuverta urgent` lists it, and
`kuverta urgent --model` asks the model first ([CLI](../developers/cli.md)).
