// Each account opts into one existing filing root. Reading a structure reads
// directory names only. Saving an attachment is a separate confirmed action.
function filingPath(path) { return String(path ?? "").replace(/^\\\\\?\\/, ""); }

let filingSettingsTicket = 0;
async function fillFilingSettings(account) {
  const ticket = ++filingSettingsTicket;
  const box = el("account-filing");
  const current = () => ticket === filingSettingsTicket && settings.editing === account;
  box.replaceChildren(node("h3", {textContent:t("Attachment filing")}));
  if (account == null) {
    box.append(node("p",{className:"hint",textContent:t("Save this account before choosing its filing folder.")}));
    return;
  }
  box.append(node("p", {className:"hint",textContent:t("Choose an existing root folder for this account. Only directory names are read. Attachments are saved only after confirmation; existing files are never overwritten.")}));
  box.append(node("p", {className:"hint",textContent:t("When you ask an online assistant for filing suggestions, relative folder names are sent to that provider too.")}));
  const input = node("input",{type:"text",id:"filing-root",placeholder:t("Full folder path")});
  const label = node("label",{textContent:t("Filing root")},input);
  const status = node("p",{className:"hint",id:"filing-status"});
  const folders = node("details",{},node("summary",{textContent:t("Folder structure")}));
  const contents = node("pre",{className:"filing-structure"});
  folders.append(contents);
  const load = async () => {
    const tree = await invoke("filing_tree",{account});
    if (!current()) return;
    input.value = filingPath(tree.root);
    status.textContent = tree.root ? t("{count} folders available.",{count:Math.max(0,tree.folders.length-1)}) : t("No filing folder connected.");
    if (tree.truncated || tree.skipped) status.textContent += " " + t("Some folders were skipped or the scan limit was reached.");
    contents.textContent = tree.folders.filter(Boolean).join("\n");
    folders.hidden = !tree.root;
  };
  const save = button(t("Connect and read folders"),async () => {
    const root = input.value.trim();
    if (!root) throw new Error(t("Enter a folder path first."));
    await invoke("set_filing_root",{account,root});
    await load();
  });
  save.id = "filing-connect";
  const disconnect = button(t("Disconnect filing folder"),async () => {
    await invoke("set_filing_root",{account,root:""});
    await load();
  });
  box.append(label,node("div",{className:"card-actions"},save,disconnect),status,folders);
  try { await load(); } catch (err) { if(current()) status.textContent=String(err); }
}

function filingSuggestionCard(proposal) {
  const email = state.accounts.find(a=>a.id===proposal.account)?.email ?? String(proposal.account);
  const path = [filingPath(proposal.root),proposal.folder,proposal.name].filter(Boolean).join(" / ");
  const {box,actions} = card(t("Suggested attachment destination"),`${email}\n${path}\n${proposal.reason || ""}`);
  box.classList.add("filing-proposal");
  cardButton(actions,t("Confirm and save"),async button => {
    if (!(await ask(t("Save a copy of {name} here?\n{path}",{name:proposal.name,path}),{yes:t("Save copy")}))) {
      button.disabled=false;
      return;
    }
    const saved = await invoke("confirm_attachment_filing",{proposal});
    button.textContent=t("Saved");
    box.classList.add("done");
    const receipt=node("p",{className:"hint",textContent:t("saved to {path}",{path:filingPath(saved)})});
    box.append(receipt);
    say(receipt.textContent);
  },"primary");
  return box;
}

// Manual filing uses the same proposal and confirmation as the assistant.
el("attachment-file").onclick = async () => {
  const {account,messageId,attachment,generation} = viewer;
  if (attachment.document != null || attachment.outgoing) return;
  const panel=el("attachment-filing");
  panel.hidden=false;
  panel.textContent=t("Loading…");
  panel.scrollIntoView({block:"nearest"});
  const current=()=>viewer.generation===generation && !attachmentSheet.hidden;
  try {
    const tree=await invoke("filing_tree",{account});
    if(!current()) return;
    if(!tree.root) { panel.textContent=t("Choose this account's filing folder in Settings first."); return; }
    const select=node("select",{id:"attachment-filing-folder"});
    for(const folder of tree.folders) select.append(node("option",{value:folder,textContent:folder || t("Root folder")}));
    panel.replaceChildren(node("label",{textContent:t("Destination folder")},select));
    if(tree.skipped || tree.truncated) panel.append(node("p",{className:"hint",textContent:t("Some folders were skipped or the scan limit was reached.")}));
    const proposalBox=node("div");
    const propose=button(t("Preview destination"),async () => {
      const proposal=await invoke("propose_attachment_filing",{account,id:messageId,index:attachment.index,folder:select.value});
      if(current()) {
        proposalBox.replaceChildren(filingSuggestionCard(proposal));
        proposalBox.scrollIntoView({block:"nearest"});
      }
    });
    select.onchange=()=>proposalBox.replaceChildren();
    panel.append(propose,proposalBox);
    panel.scrollIntoView({block:"nearest"});
  } catch(err) { if(current()) panel.textContent=String(err); }
};
