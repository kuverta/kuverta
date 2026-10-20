# Attachments in your filing folders

Under **Settings → Accounts → your account → Attachment filing**, enter an
existing absolute directory path and choose **Connect and read folders**.
Each account has its own root. Connecting reads directory names only; it does
not read existing documents or make other accounts' messages available.
Links and junctions are excluded. A large or inaccessible structure shows a
partial-scan warning. Disconnecting removes the reference, not the directory.

Open a mail attachment and choose **Save in filing folder…**, a destination,
then **Preview destination**. Review the account, name and full destination.
**Confirm and save → Save copy** writes a copy; cancelling writes nothing.
Existing files are not overwritten: a name collision gets a numbered name.
This avoids overwriting but is **not a duplicate-invoice check**. The original
attachment and mail stay in the mailbox.

The assistant can read the relative folder structure and propose an attachment
destination. It cannot confirm its own proposal. With a hosted model, the folder
names it requests and the mail/attachments it reads are sent to that provider.
Choose your intended account before starting the conversation.

## Setting up invoice collection for each profile

Open **Settings → Profiles → Collect invoices**. The first collection opens a
five-step setup for that profile: actual company and alternate spellings,
optional address and associated people; incoming/outgoing invoices and credits;
confirmation of all assigned mail accounts; an existing destination and default
period; and a final summary. Known people are not an automatic approval list:
personal invoices still require a decision about their business purpose.

Choose **Previous month**, **Previous quarter**, **Previous year** or **Custom
dates** as the default. Every collection can use different dates, including
periods across years. The wizard saves a draft when moving between steps or
choosing **Save draft and close**. Only **Confirm setup** activates the settings;
neither action reads documents, calls a model or files anything. Existing
profiles also require this confirmation. Use **Edit invoice setup** on the
collection form to revise it. Changing settings invalidates earlier preview
approvals; changing the profile's accounts also requires reconfirmation.

Review terms such as test invoices, samples and private transactions prevent
automatic selection. Company and invoice direction must be evidenced in the
document; missing evidence or a personal recipient stays for manual review.
These checks help organize review, not determine tax eligibility.

## Preparing invoices for a chosen period

Go to **Settings → Profiles → Collect invoices** beside the desired profile,
or ask the assistant: **“Sammle alle Rechnungen für Example Company aus Q3 2026.”**
The assistant resolves the named profile and invoice dates, then opens this same
joint preview. With confirmed setup and a saved profile destination, preview and
text analysis start automatically; otherwise complete setup and start explicitly
from the collection form. A chat card
keeps the profile and dates so you can reopen the preview later. Opening an old
conversation does not rerun it. If another dialog is open, use the card after
closing that dialog. Missing or ambiguous profile/year should be clarified.
The collection combines **all mail accounts assigned to that one profile**;
it never selects all profiles or silently changes their memberships. Check the
account list before proceeding. Example Company and Another Company remain
separate when configured as separate profiles, even if their filing folders are near
each other. Each profile remembers its own destination and export history.

1. Enter an **existing destination directory** for this collection, for example
   your profile's `Invoices` directory. The chosen folder layout controls whether
   approved copies go directly there or into year/quarter/month subfolders.
   Unrelated documents already in the directory are not indexed.
2. Choose a period preset or any start and end dates (Q3 2026:
   **2026-07-01 to 2026-09-30**) and separately
   the mail search period. The initial search extends the chosen invoice period by
   31 days at both ends; widen it for invoices delivered much later. Mail dates
   are interpreted as UTC day boundaries. Messages without a date are included.
3. Choose **Build joint preview**. It scans stored metadata in batches, reads
   attachment-bearing messages locally and shows PDF, image, text and XML
   candidates, including files whose names do not mention invoices. With
   **Suggest document dates and recognize invoice versions with AI** enabled,
   the configured assistant reads document text and suggests issuer, recipient,
   invoice number, document date, gross amount/currency and correction wording.
   The form identifies the provider; hosted document text leaves this computer
   and may incur API charges. No files are copied at this stage. Scans and image
   documents remain manual review items; scan settings are not changed.
4. Review the common preview. Supported invoices and credits matching the
   profile setup and inside the period are preselected with their dates;
   missing/uncertain facts, possible versions
   and documents outside the period are not. **Evidence from the document**
   shows the source excerpts. Use **View document** to check a doubtful case,
   adjust **Include document**, **Verified document date** or **Document type**
   as needed. The date must be inside the accounting period. These are model
   suggestions, not certification of an invoice's contents or authenticity.
   You can read the source mail without changing its read flag. Sender, account,
   subject and mail date remain visible for each candidate.
   The initial **Invoices in period** filter separates recognized invoices and
   credits from **Other documents**, **Outside invoice period**, **Duplicates**
   and **Needs review** (including unknown dates). Filter counts cover the whole
   preview; **Selected documents** shows the proposed batch. Other documents
   remain available for inspection and correction, but are never auto-selected.
5. **Review and save selection → Confirm collection** confirms the joint
   selection and destination. Copies and a JSON export log are written there.
   Mail is never moved, deleted, marked read or sent by this workflow.

**Duplicate checks:** SHA-256 identifies identical attachment bytes across the
profile's accounts and previous successful copies, regardless of filename.
The same bytes in another profile are not suppressed. Different contents with
the same filename get a review warning. Supported issuer/number/type facts also
group possible versions with different bytes, even across filenames/accounts,
and flag matches against earlier analyzed exports in this profile. Different
amounts and explicit correction wording remain visible. These are **possible
versions**, not proven duplicates: choose the appropriate version in the preview;
Kuverta never silently picks the newest mail or overwrites an older document.
Credit notes remain distinct from invoices. Unreadable or differently spelled
identities can still require manual comparison. Successful-copy
history remains authoritative if files are subsequently moved or deleted outside
Kuverta; it does not silently recreate them in another directory.

In the invoice wizard, **Choose folder…** opens the native Windows folder chooser;
typing a path remains available on all platforms. Picking or cancelling does not
start a scan, confirm setup or copy documents. Choose an existing base directory.
**Folder structure** offers a flat layout or **Year → quarter → month**, for
example `2026/Q3/09`. New setups suggest the latter; existing configurations keep
their flat layout until edited. This is independent of the collection period:
a custom range spanning years still routes each document by its verified invoice
date. Each review card and the final confirmation show the destination folders.

Existing date folders with these exact names are reused. Kuverta creates missing
date folders only after **Confirm collection**, never while setting up or reviewing.
Other folder names, such as `September` instead of `09`, are not guessed or renamed.
No existing documents are indexed or reorganized; duplicate detection covers
Kuverta's collection history, not arbitrary files previously filed manually.
Links/junctions and files occupying a required directory name block filing.
The JSON export log remains in the base directory.

Files have a date/profile/hash prefix to keep names distinct. Existing files are
never overwritten, and Windows origin marking is retained. A durable reservation
is stored before each copy. After interruption, a new preview retries pending
copies at their reserved paths, checking identical contents before accepting
them. A conflicting file is reported as a failure. If the destination changed
while a copy was pending, restore its original destination to complete the retry.

The receipt and **Previous collections** show saved/skipped/failed files. The
JSON log includes the selected profile/accounts, search bounds, outcomes,
document dates/types, analysis facts and unselected candidates/notices. If writing the log fails,
the receipt still exists in the profile's database history. Deleting a profile
removes its history and duplicate receipts but leaves exported files on disk.

**Coverage limits:** only downloaded mail is checked, across all its folders
(including sent, spam and trash), not Paperless or remote portals. Invoice-like
subjects/snippets without attachments are shown as missing-document/portal
notices; this does not find every portal invoice. Encrypted, missing or >30 MiB
messages are flagged. Risky/unsupported attachments are not selectable. A preview
stops at 2,000 candidates/notices and requires a narrower mail period before
export; each confirmation is limited to 500 documents / 512 MiB. The progress
count describes stored messages checked, not proof of complete invoice coverage.

Analysis reads up to 32,000 characters of document text per unique file and
flags truncated text for review. Up to 100 new files are analyzed per pass;
**Analyze remaining documents** continues after that. Results are cached by
profile and exact file bytes so duplicate attachments and later previews do not
repeat model requests. Manual changes to the selection survive another pass.
English ordinal dates such as `1st September 2026` are validated against the
original document. Cached results affected by the former ordinal-date defect
are revalidated locally when analysis runs, without another provider request.
Explicit selection, date, type and reason changes are saved to the local database
for the **exact document bytes in this profile**. Wait for **Correction saved for
this document and profile**. Failed saves remain visible and block export and
closing until retried or reset. **Continue saved review** reopens the last
completed, still-valid preview without scanning mail or making model requests.
Corrections also apply to matching files in later previews and survive restarting
Kuverta. They do not apply to other profiles, different document versions or every
invoice from a sender. An unchanged automatic checkmark is not user feedback.

Use **Use suggestion again** to remove a correction; its local audit history is
retained. After changing confirmed setup, earlier corrections require a new
explicit decision. Old selections from versions before this feature cannot be
recovered. Broad learning and suggested sender rules are not implemented; the
stored feedback does not train a model. Each export still needs its own joint
preview and final approval. Check business purpose and uncertain identities
yourself. Saving copies does not send them to DATEV.
No OCR requests are made for scans. With AI disabled, the same preview supports
manual dates and selection. Provider failures are shown and preserve the preview.

The assistant's collection tool only hands the profile and period to this review
flow. It has no tool to confirm the batch: saying “yes” in chat does not replace
**Review and save selection → Confirm collection**. Its separate filing tool
still proposes individual account-scoped copies. Scheduled collection remains
future work; no recurring filesystem export is created by mail tasks.

An **IMAP folder** holds messages and appears in other mail clients. A **filing
directory** holds attachment copies on your computer. Specify which you mean
when asking to put something in a folder.
