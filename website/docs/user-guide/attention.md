---
description: Needs attention ranks your Inbox by how soon each message needs you, worked out by kuverta's urgency agent from facts it checks and, if you like, your model's judgement.
---

# Needs attention

**Needs attention**, at the top of the sidebar, ranks the mail in your Inbox by
how soon it needs you. The number beside it counts the messages that need you
this week or sooner. Each row gives the reason in one sentence, with what to do
and by when.

![Needs attention: three messages ranked, each with its reason](../assets/screens/attention-light.webp#only-light)
![Needs attention: three messages ranked, each with its reason](../assets/screens/attention-dark.webp#only-dark)

| Shown | Means |
| --- | --- |
| **today** | Act today or tomorrow: a deadline is close, a bill is overdue, someone is blocked. |
| **this week** | Act in the next few days. |
| *can wait* and *nothing to do* | Not listed here. |

After the level comes the action (*reply*, *pay*, *attend*, *decide* or
*read*) and a deadline when the message names one. An open message shows the
same verdict under its subject, with who judged it.

## How it is worked out

A small agent reads the Inbox mail of the last three weeks that is not bulk
mail. Newsletters, marketing and notifications are left to
[Cleanup](cleanup.md). For each message it first checks what kuverta can find
out for itself:

- whether you have written to this sender before, and how often;
- whether you have **already answered** it, in which case it needs nothing;
- how many messages this sender has sent you before;
- whether it was written to you directly or to a group or a list;
- whether it talks about deadlines or asks for payment.

From these facts it forms a first verdict for every message at once.

Then, if you let it, it asks the model chosen for **sorting mail**
([Models](models.md)) about each message in turn. The model gets the facts
above and the start of the message, and answers with how soon, what to do, by
when and why. Its answer replaces the rules' verdict for that message. Two
rules keep the model in check:

- A named deadline decides the level. A small model calls a bill due in two
  weeks "today" as readily as one due tomorrow, so kuverta counts the date
  itself. Overdue or within a day is *today*, within a week *this week*, later
  *can wait*.
- The message is treated as data. "URGENT: reply now" is often advertising, so
  the model is told that a sender's own claim of urgency is weak evidence and
  to lean on the facts. The message is fenced so that nothing in it can pose as
  an instruction.

Each message is judged once and the verdict is kept. The agent runs quietly
after every sync, and **Ask the model** in the list header runs it now.

## What leaves your computer

The rules run on your computer. The model runs where you chose. With the
default, Ollama on this computer, nothing leaves it. With a hosted service,
the start of each new message that is not bulk mail is sent to it, with the
checked facts. To keep the rules only, untick **After each sync, let the model
for sorting mail judge how soon new mail needs me** in **Settings → General**.

## For an assistant, and on the command line

The [MCP server](../developers/mcp.md) offers the same ranking as the
`urgent_messages` tool, so an assistant asked "what should I deal with first?"
answers from it. On the command line, `kuverta urgent` lists it, and
`kuverta urgent --model` asks the model first ([CLI](../developers/cli.md)).
