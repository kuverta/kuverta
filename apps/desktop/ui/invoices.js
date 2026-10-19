// A collection is reviewed across ONE profile. Its immutable preview lives in
// the core; the shell sends only selected row numbers, document dates and kind.
const invoiceSheet = node("div", {id:"invoice-sheet", className:"sheet dialog", hidden:true});
invoiceSheet.innerHTML = `<section class="dialog-panel invoice-panel" role="dialog" aria-modal="true" aria-labelledby="invoice-title">
  <div class="dialog-head"><h2 id="invoice-title"></h2><p id="invoice-accounts" class="hint"></p></div>
  <div class="dialog-body" id="invoice-body"></div>
  <div class="dialog-foot"><span id="invoice-count" role="status" class="grow"></span><button id="invoice-export" class="primary" hidden></button><button id="invoice-close"></button></div>
</section>`;
document.body.append(invoiceSheet);
const invoiceState = {generation:0,busy:false,preview:null,selection:new Map(),reviewed:new Set(),modelNotice:"",wizardClose:null,reviewQueue:Promise.resolve(),unsaved:new Set()};
el("invoice-close").textContent=t("Close");
el("invoice-close").onclick=()=>{if(!invoiceState.busy)closeDialog(invoiceSheet);};
invoiceSheet.addEventListener("close",()=>{invoiceState.generation++;invoiceState.wizardClose=null;});
// An export may finish after closing the dialog; its durable receipt is still
// available under this profile's history. Never pretend closing cancels writes.
invoiceSheet.addEventListener("keydown",event=>{
  if(event.key==="Escape"&&invoiceState.wizardClose){event.preventDefault();event.stopPropagation();void invoiceState.wizardClose();return;}
  if(event.key==="Escape"&&invoiceState.unsaved.size){event.preventDefault();event.stopPropagation();say(t("Save the pending corrections before closing."),true);return;}
  if(invoiceState.busy && event.key==="Escape") {event.preventDefault();event.stopPropagation();}
},true);

function invoiceDate(value) { return `${value.getFullYear()}-${String(value.getMonth()+1).padStart(2,"0")}-${String(value.getDate()).padStart(2,"0")}`; }
function invoiceStatus() {
  const preview=invoiceState.preview;
  if(!preview)return;
  const selected=[...invoiceState.selection.values()];
  for(const option of el("invoice-filter")?.options??[]) {
    const count=preview.rows.filter((row,index)=>invoiceFilterMatches(row,index,option.value,preview)).length;
    option.textContent=`${option.dataset.label} (${count})`;
  }
  const valid=selected.length>0 && selected.every(s=>s.invoice_date>=preview.request.from && s.invoice_date<=preview.request.to);
  el("invoice-count").textContent=t("{count} selected · document dates must be reviewed",{count:selected.length});
  el("invoice-export").hidden=false;
  el("invoice-export").disabled=!valid || !preview.complete || preview.limited || invoiceState.busy || invoiceState.unsaved.size>0;
  el("invoice-export").textContent=t("Review and save selection");
}

async function openAssistantInvoiceCollection(proposal) {
  if(invoiceState.busy || !invoiceSheet.hidden) {
    say(t("Finish or close the current invoice preview first."),true);return;
  }
  // Old chat cards must resolve today's profile membership, not a saved list.
  state.profiles=await invoke("profiles");
  if(!state.profiles.some(p=>p.id===proposal.profile))throw Error(t("This profile no longer exists. Request a new invoice preview."));
  await openInvoiceCollection(proposal.profile,{from:proposal.from,to:proposal.to,autoStart:true});
}

async function openInvoiceCollection(profileId, options={}) {
  if(invoiceState.busy||invoiceState.unsaved.size)return;
  const requestedDate=value=>{
    const date=new Date(`${value}T12:00:00`);
    if(!/^\d{4}-\d{2}-\d{2}$/.test(value) || !Number.isFinite(date.getTime()) || invoiceDate(date)!==value)throw Error(t("Invalid invoice period. Request a new preview."));
    return date;
  };
  const now=new Date();
  let first=options.from!=null?requestedDate(options.from):new Date(now.getFullYear(),Math.floor(now.getMonth()/3)*3-3,1);
  let last=options.to!=null?requestedDate(options.to):new Date(first.getFullYear(),first.getMonth()+3,0);
  if(last<first)throw Error(t("Invalid invoice period. Request a new preview."));
  const generation=++invoiceState.generation;
  invoiceState.preview=null;invoiceState.selection.clear();invoiceState.reviewed.clear();invoiceState.modelNotice="";invoiceState.wizardClose=null;
  el("invoice-close").disabled=false;el("invoice-close").textContent=t("Close");el("invoice-close").onclick=()=>{if(!invoiceState.busy&&!invoiceState.unsaved.size)closeDialog(invoiceSheet);else if(invoiceState.unsaved.size)say(t("Save the pending corrections before closing."),true);};
  const profile=state.profiles.find(p=>p.id===profileId);
  if(!profile)return;
  invoiceSheet.hidden=false;
  el("invoice-title").textContent=`${t("Collect invoices")} · ${profile.name}`;
  el("invoice-accounts").textContent="";
  el("invoice-count").textContent="";
  el("invoice-export").hidden=true;
  el("invoice-body").textContent=t("Loading…");
  try {
    const settings=await invoke("invoice_settings",{profile:profileId});
    if(generation!==invoiceState.generation)return;
    el("invoice-accounts").textContent=state.accounts.filter(a=>profile.accounts.includes(a.id)).map(a=>a.email).join(" · ");
    if(options.setup||settings.setup?.needs_confirmation||!settings.setup?.confirmed){await renderInvoiceSetup(profile,settings,options);return;}
    const setup=settings.setup.confirmed;
    if(options.from==null&&options.to==null){
      if(setup.period==="custom"){first=requestedDate(setup.custom_from);last=requestedDate(setup.custom_to);}
      else [first,last]=invoicePeriodBounds(setup.period,now);
    }
    const body=el("invoice-body");body.replaceChildren();
    if(settings.resumable&&!options.autoStart){
      const resume=button(t("Continue saved review ({from} – {to})",{from:settings.resumable.request.from,to:settings.resumable.request.to}),()=>{invoiceState.preview=settings.resumable;suggestInvoiceSelection();renderInvoicePreview();});resume.id="invoice-resume";body.append(resume);
    }
    body.append(button(t("Edit invoice setup"),()=>openInvoiceCollection(profileId,{...options,setup:true,autoStart:false})));
    body.append(node("p",{textContent:t("Collect copies from all mail accounts in this profile. Review invoice dates and credit notes together before saving. Nothing changes in your mailboxes.")}));
    const form=node("form",{id:"invoice-form"});
    const start=new Date(first);start.setDate(start.getDate()-31);
    const end=new Date(last);end.setDate(end.getDate()+31);
    const field=(id,label,type,value)=>{
      const input=node("input",{id,type,value,required:true});
      return {input,label:node("label",{textContent:t(label)},input)};
    };
    const root=field("invoice-root","Profile invoice destination (existing folder)","text",filingPath(settings.root));
    root.label.append(invoiceFolderPicker(root.input));
    const from=field("invoice-from","Invoice date from","date",invoiceDate(first));
    const to=field("invoice-to","Invoice date to","date",invoiceDate(last));
    const mailFrom=field("invoice-mail-from","Search mail dated from","date",invoiceDate(start));
    const mailTo=field("invoice-mail-to","Search mail dated to","date",invoiceDate(end));
    const period=node("select",{id:"invoice-period"});invoicePeriodOptions(period);period.value=options.from!=null?"custom":setup.period;
    const searchMargin=()=>{const left=requestedDate(from.input.value),right=requestedDate(to.input.value);left.setDate(left.getDate()-31);right.setDate(right.getDate()+31);mailFrom.input.value=invoiceDate(left);mailTo.input.value=invoiceDate(right);};
    period.onchange=()=>{if(period.value!=="custom"){const [left,right]=invoicePeriodBounds(period.value);from.input.value=invoiceDate(left);to.input.value=invoiceDate(right);searchMargin();}};
    from.input.onchange=to.input.onchange=()=>{period.value="custom";};
    form.append(node("label",{textContent:t("Collection period")},period));
    form.append(root.label,node("div",{className:"invoice-fields"},from.label,to.label,mailFrom.label,mailTo.label));
    form.append(node("p",{className:"hint",textContent:`${t("Folder structure")}: ${invoiceLayoutLabel(setup.layout)}`}));
    form.append(button(t("Adjust mail search to these dates (31-day margin)"),()=>{try{searchMargin();}catch(error){say(String(error),true);}}));
    form.append(node("p",{className:"hint",textContent:t("Mail dates only limit the search. Include later deliveries if necessary. Only downloaded mail is checked, including sent mail, spam and trash. Paper mail is not included.")}));
    const analyze=node("input",{type:"checkbox",id:"invoice-analyze",checked:true});
    form.append(node("label",{className:"check",textContent:t("Suggest document dates and recognize invoice versions with AI")},analyze));
    const models=node("p",{className:"hint",textContent:t("Loading model settings…")});form.append(models);
    let modelSettingsReady=false;
    const modelsReady=Promise.all([invoke("ai_tasks"),invoke("ai_providers")]).then(([tasks,providers])=>{
      if(!form.isConnected || generation!==invoiceState.generation)return;
      const names=["assistant"].map(task=>{const job=tasks.find(j=>j.task===task);const provider=providers.find(p=>p.id===job?.provider_id);return {job,provider};});
      if(names.some(n=>!n.job || !n.provider))throw Error(t("Check the model settings before analyzing documents."));
      const hosted=names.some(n=>!n.provider.local);
      models.className=hosted?"error":"hint";
      models.textContent=names.map(n=>`${n.provider.label}: ${n.job.model}`).join(" · ")+". "+t(hosted?"Document text is sent to the configured assistant service. API costs may apply. Up to 100 new documents per pass; repeated files use cached results.":"Document analysis runs with the configured local models. Up to 100 new documents per pass; repeated files use cached results.")+" "+t("Scans and image documents are left for manual review.");
      invoiceState.modelNotice=models.textContent;
      modelSettingsReady=true;
    }).catch(error=>{models.textContent=String(error);analyze.checked=false;analyze.disabled=true;});
    const startButton=node("button",{type:"submit",className:"primary",textContent:t("Build joint preview")});
    form.append(startButton);body.append(form);
    const history=node("details",{},node("summary",{textContent:t("Previous collections ({count})",{count:settings.exports.length})}));
    for(const result of settings.exports)history.append(invoiceReceipt(result));
    body.append(history);
    const buildPreview=async()=>{
      if(invoiceState.busy)return;
      if(analyze.checked&&!modelSettingsReady){say(t("Loading model settings…"));return;}
      invoiceState.busy=true;startButton.disabled=true;el("invoice-close").textContent=t("Stop preview");
      // Closing stops BETWEEN batches and never writes attachments.
      el("invoice-close").onclick=()=>closeDialog(invoiceSheet);
      const progress=node("p",{id:"invoice-progress",role:"status"});form.append(progress);
      try {
        let preview=await invoke("start_invoice_collection",{request:{profile:profileId,root:root.input.value,from:from.input.value,to:to.input.value,mail_from:mailFrom.input.value,mail_to:mailTo.input.value}});
        while(!preview.complete && !preview.limited && generation===invoiceState.generation) {
          progress.textContent=t("Checking mail: {done} / {total}",{done:preview.scanned,total:preview.total});
          preview=await invoke("scan_invoice_collection",{token:preview.token});
        }
        if(generation!==invoiceState.generation)return;
        invoiceState.preview=preview;invoiceState.selection.clear();
        if(analyze.checked)await analyzeInvoicePreview(progress,generation);
        if(generation!==invoiceState.generation)return;
        suggestInvoiceSelection();renderInvoicePreview();
      } catch(error) {
        if(generation===invoiceState.generation) {
          if(invoiceState.preview?.complete) {suggestInvoiceSelection();renderInvoicePreview();el("invoice-body").prepend(node("p",{className:"error",textContent:String(error)}));}
          else progress.textContent=String(error);
        }
      }
      finally {
        invoiceState.busy=false;startButton.disabled=false;
        el("invoice-close").textContent=t("Close");el("invoice-close").onclick=()=>{if(!invoiceState.busy&&!invoiceState.unsaved.size)closeDialog(invoiceSheet);else if(invoiceState.unsaved.size)say(t("Save the pending corrections before closing."),true);};
        invoiceStatus();
      }
    };
    form.onsubmit=event=>{event.preventDefault();void buildPreview();};
    root.input.focus();
    if(options.autoStart) {
      await modelsReady;
      if(generation!==invoiceState.generation)return;
      if(root.input.value.trim() && modelSettingsReady)await buildPreview();
      else if(!root.input.value.trim())body.prepend(node("p",{className:"hint",textContent:t("Choose this profile's destination to build the joint preview. Files are saved only after your separate approval.")}));
    }
  }catch(error){el("invoice-body").textContent=String(error);}
}

async function analyzeInvoicePreview(progress,generation) {
  let done=0;
  const seen=new Set();
  for(let index=0;index<invoiceState.preview.rows.length && done<100;index++) {
    if(generation!==invoiceState.generation)return;
    const row=invoiceState.preview.rows[index];
    if(row.issue || (row.analysis&&!row.analysis_needs_refresh) || row.analysis_error==="invoice_text_unavailable" || !row.digest || seen.has(row.digest))continue;
    seen.add(row.digest);done++;
    progress.textContent=t("Reading document {number} (up to 100 per pass)…",{number:done});
    const preview=await invoke("analyze_invoice_document",{token:invoiceState.preview.token,row:index});
    if(generation!==invoiceState.generation)return;
    invoiceState.preview=preview;
    // A failed provider or unreadable file stays visible. Stop this pass
    // instead of repeatedly spending requests against the same failure.
    if(preview.rows[index].analysis_error && preview.rows[index].analysis_error!=="invoice_text_unavailable")break;
  }
}
function suggestInvoiceSelection() {
  const preview=invoiceState.preview;
  preview.rows.forEach((row,index)=>{
    if(invoiceState.reviewed.has(index))return;
    const a=row.analysis;
    if(row.review){
      const r=row.review;
      if(r.setup_revision===preview.setup_revision&&r.decision==="include"&&!row.issue&&!row.previous_path&&row.duplicate_of==null&&r.invoice_date>=preview.request.from&&r.invoice_date<=preview.request.to)invoiceState.selection.set(index,{row:index,invoice_date:r.invoice_date,kind:r.kind});else invoiceState.selection.delete(index);
      return;
    }
    if(a?.supported && row.eligibility?.status==="suggested" && !row.analysis_needs_refresh && !row.issue && !row.previous_path && row.duplicate_of==null
      && !row.versions?.length && !row.previous_versions?.length && a.invoice_date>=preview.request.from && a.invoice_date<=preview.request.to) {
      invoiceState.selection.set(index,{row:index,invoice_date:a.invoice_date,kind:a.kind});
    } else invoiceState.selection.delete(index);
  });
}

function invoiceFilterMatches(row,index,filter,preview) {
  const correction=row.review?.setup_revision===preview.setup_revision?row.review:null;
  const a=correction?.invoice_date?{...row.analysis,kind:correction.kind,invoice_date:correction.invoice_date}:row.analysis,duplicate=Boolean(row.previous_path)||row.duplicate_of!=null;
  const other=a?.kind==="other",outside=Boolean(a?.invoice_date)&&(a.invoice_date<preview.request.from||a.invoice_date>preview.request.to);
  const invoice=["invoice","credit"].includes(a?.kind);
  const review=!duplicate&&((row.review&&row.review.setup_revision!==preview.setup_revision)||correction?.decision==="review"||(!other&&!outside&&!row.review&&(row.eligibility?.status==="review"||row.issue||row.analysis_error||row.analysis_needs_refresh||!a?.supported||row.versions?.length||row.previous_versions?.length)));
  return filter==="all" || (filter==="invoices"&&!row.issue&&!duplicate&&invoice&&Boolean(a.invoice_date)&&!outside)
    || (filter==="selected"&&invoiceState.selection.has(index)) || (filter==="other"&&other)
    || (filter==="excluded"&&(row.review?.decision==="exclude"||(!row.review&&row.eligibility?.status==="excluded")))
    || (filter==="outside"&&!other&&outside) || (filter==="new"&&!row.issue&&!duplicate)
    || (filter==="review"&&review) || (filter==="issues"&&row.issue) || (filter==="duplicates"&&duplicate);
}

function renderInvoicePreview(keepView=false) {
  const oldFilter=el("invoice-filter")?.value,oldSearch=el("invoice-search")?.value,oldScroll=el("invoice-body").scrollTop;
  const preview=invoiceState.preview,body=el("invoice-body");body.replaceChildren();
  el("invoice-accounts").textContent=preview.accounts.map(a=>a.email).join(" · ");
  body.append(node("p",{className:"invoice-destination",textContent:`${preview.request.from} – ${preview.request.to} → ${filingPath(preview.request.root)}`}));
  body.append(node("p",{className:"hint",textContent:`${t("Folder structure")}: ${invoiceLayoutLabel(preview.layout)}`}));
  body.append(node("p",{textContent:t("{scanned} of {total} stored messages checked · {rows} candidates and notices. This is a review list, not proof that all invoices were found.",{scanned:preview.scanned,total:preview.total,rows:preview.rows.length})}));
  body.append(node("p",{className:"hint",textContent:t("Review the suggested dates, amounts and selection together. Supported invoices inside the period are preselected. Possible versions, uncertain documents and missing dates need your decision. AI suggestions can be wrong; nothing is saved until you confirm.")}));
  if(preview.limited)body.append(node("p",{className:"error",textContent:t("Preview limit reached. Narrow the mail dates and start again before saving.")}));
  const toolbar=node("div",{className:"card-actions"});
  const search=node("input",{type:"search",id:"invoice-search",placeholder:t("Filter by sender, subject or filename")});
  const filter=node("select",{id:"invoice-filter"});
  for(const [value,text] of [["invoices","Invoices in period"],["selected","Selected documents"],["review","Needs review"],["excluded","Excluded documents"],["other","Other documents"],["outside","Outside invoice period"],["issues","Missing or unsupported documents"],["duplicates","Duplicates"],["new","New documents"],["all","All candidates"]]) {
    const count=preview.rows.filter((row,index)=>invoiceFilterMatches(row,index,value,preview)).length;
    const option=new Option(`${t(text)} (${count})`,value);option.dataset.label=t(text);filter.append(option);
  }
  filter.value=preview.rows.some(r=>r.analysis)?"invoices":"review";
  if(keepView&&oldFilter){filter.value=oldFilter;search.value=oldSearch??"";}
  body.append(node("p",{className:"hint",textContent:t("Invoices in period shows recognized invoices and credits. Use Needs review for unclear or missing dates; other documents, other periods and duplicates have their own filters. Nothing is discarded.")}));
  toolbar.append(search,filter,button(t("Start again"),()=>openInvoiceCollection(preview.request.profile,{from:preview.request.from,to:preview.request.to})));
  const pending=preview.rows.filter(r=>r.digest&&!r.issue&&(!r.analysis||r.analysis_needs_refresh)&&r.analysis_error!=="invoice_text_unavailable"&&r.duplicate_of==null).length;
  if(pending) {
    const again=button(t("Analyze remaining documents ({count})",{count:pending}),async()=>{
      if(invoiceState.busy)return;
      if(!await ask(invoiceState.modelNotice || t("Check the model settings before analyzing documents."),{yes:t("Analyze documents")}))return;
      invoiceState.busy=true;again.disabled=true;invoiceStatus();
      const status=node("p",{role:"status"});body.prepend(status);
      const generation=invoiceState.generation;
      try{await analyzeInvoicePreview(status,generation);if(generation===invoiceState.generation){suggestInvoiceSelection();renderInvoicePreview();}}
      catch(error){say(String(error),true);}
      finally{invoiceState.busy=false;again.disabled=false;invoiceStatus();}
    });toolbar.append(again);
  }
  body.append(toolbar);
  const list=node("div",{id:"invoice-rows"});body.append(list);
  const navigation=node("div",{className:"card-actions"});body.append(navigation);
  let page=0;
  const draw=()=>{
    list.replaceChildren();navigation.replaceChildren();
    const query=search.value.toLowerCase();
    const matches=preview.rows.map((row,index)=>({row,index})).filter(({row,index})=>{
      return invoiceFilterMatches(row,index,filter.value,preview) && `${row.sender} ${row.subject} ${row.attachment?.name??""} ${row.analysis?.issuer??""} ${row.analysis?.number??""}`.toLowerCase().includes(query);
    });
    for(const {row,index} of matches.slice(page*40,(page+1)*40))list.append(invoiceRow(row,index));
    if(!matches.length)list.append(node("p",{textContent:t("No matching documents.")}));
    const back=button(t("Previous"),()=>{page--;draw();});back.disabled=page===0;
    const next=button(t("Next"),()=>{page++;draw();});next.disabled=(page+1)*40>=matches.length;
    navigation.append(back,node("span",{textContent:`${Math.min(matches.length,page*40+1)}–${Math.min(matches.length,(page+1)*40)} / ${matches.length}`}),next);
  };
  search.oninput=filter.onchange=()=>{page=0;draw();};draw();invoiceStatus();
  if(keepView)body.scrollTop=oldScroll;
}
function invoiceEligibilityText(code) {
  return t(({invoice_facts_missing:"Document facts need review",invoice_not_invoice:"Not an invoice or credit note",invoice_review_term:"A configured review term was found",invoice_person_review:"Personal recipient: confirm the business purpose",invoice_party_unclear:"Company or invoice direction is not established",invoice_address_unconfirmed:"The company address is not evidenced",invoice_type_excluded:"This document direction or type is excluded by profile setup",invoice_profile_matches:"The parties match this profile"})[code]??"Document facts need review");
}
function invoiceRow(row,index) {
  const selected=invoiceState.selection.get(index);
  const box=node("article",{className:"invoice-row"});box.dataset.row=index;
  const email=invoiceState.preview.accounts.find(a=>a.id===row.account)?.email??String(row.account);
  box.append(node("strong",{textContent:`${index+1}. ${row.attachment?.name || row.subject}`}),node("p",{className:"hint",textContent:`${email} · ${row.sender} · ${row.mail_date==null?t("Unknown mail date"):new Date(row.mail_date*1000).toLocaleDateString()}\n${row.subject}`}));
  if(row.issue)box.append(node("p",{className:"invoice-warning",textContent:t(({no_attachment:"No attached document. Check the mail for a portal link or a missing receipt.",unavailable:"Body unavailable, encrypted or too large (over 30 MiB). Check this mail separately.",unsupported:"This file format is not supported for invoice collection."})[row.issue]??row.issue)}));
  if(row.previous_path)box.append(node("p",{className:"hint",textContent:t("Already collected for this profile: {path}",{path:filingPath(row.previous_path)})}));
  else if(row.duplicate_of!=null)box.append(node("p",{className:"hint",textContent:t("Identical to candidate {number}; select that copy only.",{number:row.duplicate_of+1})}));
  if(row.same_name)box.append(node("p",{className:"invoice-warning",textContent:t("Same filename, different contents: check whether this is another copy of the same invoice.")}));
  const a=row.analysis;
  if(row.analysis_error)box.append(node("p",{className:"invoice-warning",textContent:row.analysis_error==="invoice_text_unavailable"?t("No readable document text. Scans and images are not analyzed; please review manually."):t("Automatic analysis failed; review manually or retry.")+" "+row.analysis_error}));
  if(a) {
    box.append(node("p",{className:"hint",textContent:t("Document type")+": "+t(({invoice:"Invoice",credit:"Credit note",other:"Other document",uncertain:"Uncertain document type"})[a.kind]??"Uncertain document type")}));
    box.append(node("p",{textContent:[a.issuer,a.number,a.invoice_date,[a.total,a.currency].filter(Boolean).join(" "),a.recipient,a.version].filter(Boolean).join(" · ")}));
    box.append(node("p",{className:a.supported?"hint":"invoice-warning",textContent:`${a.reason} · ${a.model}`}));
    const evidence=node("details",{},node("summary",{textContent:t("Evidence from the document")}));
    for(const quote of [a.date_quote,a.issuer_quote,a.recipient_quote,a.number_quote,a.total_quote,a.version_quote].filter(Boolean))evidence.append(node("blockquote",{textContent:quote}));
    box.append(evidence);
    if(a.invoice_date&&(a.invoice_date<invoiceState.preview.request.from||a.invoice_date>invoiceState.preview.request.to))box.append(node("p",{className:"invoice-warning",textContent:t("The detected document date is outside the accounting period.")}));
  }
  if(a)box.append(node("p",{className:"invoice-parties",textContent:`${t("Issuer")}: ${a.issuer||t("Unknown")} → ${t("Invoice recipient")}: ${a.recipient||t("Unknown")}`}));
  if(row.eligibility?.reason)box.append(node("p",{className:row.eligibility.status==="suggested"?"hint":"invoice-warning",textContent:`${t(({incoming:"Incoming invoice",outgoing:"Outgoing invoice",unknown:"Direction needs review"})[row.eligibility.direction]??"Direction needs review")} · ${invoiceEligibilityText(row.eligibility.reason)}`}));
  if(row.review&&row.review.setup_revision!==invoiceState.preview.setup_revision)box.append(node("p",{className:"invoice-warning",textContent:t("Profile setup changed. Confirm this saved correction again.")}));
  if(row.versions?.length)box.append(node("p",{className:"invoice-warning",textContent:t("Possible versions of the same invoice: candidates {numbers}. Compare date, amount and correction notes; choose the appropriate version.",{numbers:row.versions.map(i=>i+1).join(", ")})}));
  for(const path of row.previous_versions??[])box.append(node("p",{className:"invoice-warning",textContent:t("A possible version was already collected: {path}",{path:filingPath(path)})}));
  const actions=node("div",{className:"invoice-fields"});
  if(row.attachment && !row.issue)actions.append(button(t("View document"),()=>openAttachment(row.account,row.message,row.attachment)));
  if(!row.issue && !row.previous_path && row.duplicate_of==null) {
    const check=node("input",{type:"checkbox",checked:Boolean(selected)});
    check.setAttribute("aria-label",t("Include document"));
    const when=node("input",{type:"date",value:row.review?.invoice_date??selected?.invoice_date??a?.invoice_date??"",min:invoiceState.preview.request.from,max:invoiceState.preview.request.to});
    const destination=node("p",{className:"hint invoice-row-destination"});
    const showDestination=()=>{destination.textContent=`${t("Destination folder")}: ${invoiceFolderForDate(invoiceState.preview,when.value)}`;};showDestination();box.append(destination);
    const kind=node("select");kind.append(new Option(t("Invoice"),"invoice"),new Option(t("Credit note"),"credit"));kind.value=row.review?.kind??selected?.kind??(a?.kind==="credit"?"credit":"invoice");
    const reason=node("input",{type:"text",className:"invoice-correction-reason",value:row.review?.reason??"",placeholder:t("For example: private transaction or test invoice"),maxLength:500});reason.setAttribute("aria-label",t("Correction reason"));
    const status=node("p",{className:"hint invoice-save-status",textContent:row.review?t("Correction saved for this document and profile"):t("Changing the selection saves your correction for this document and profile.")});status.setAttribute("role","status");
    let edit=0,persisted=row.review;
    const save=(review,reset=false)=>{
      const serial=++edit,generation=invoiceState.generation,token=invoiceState.preview.token;
      invoiceState.unsaved.add(index);invoiceState.reviewed.add(index);status.className="hint invoice-save-status";status.textContent=t("Saving…");invoiceStatus();
      invoiceState.reviewQueue=invoiceState.reviewQueue.catch(()=>{}).then(async()=>{
        try {
          const preview=await invoke("save_invoice_review",{token,row:index,review});
          if(generation!==invoiceState.generation)return;
          invoiceState.preview=preview;
          if(serial===edit){persisted=preview.rows[index].review;invoiceState.unsaved.delete(index);invoiceState.reviewed.delete(index);suggestInvoiceSelection();status.textContent=t(reset?"Correction removed":"Correction saved for this document and profile");if(reset)renderInvoicePreview(true);}
        }catch(error){if(serial===edit){status.className="error invoice-save-status";status.textContent=t("Could not save. Your changes are still open.")+" "+String(error);}}
        finally{invoiceStatus();}
      });
    };
    const change=event=>{
      showDestination();
      invoiceState.reviewed.add(index);
      if(check.checked)invoiceState.selection.set(index,{row:index,invoice_date:when.value,kind:kind.value});else invoiceState.selection.delete(index);
      const decision=check.checked?"include":event.target===check?"exclude":persisted?.decision==="exclude"?"exclude":"review";
      if(!reason.value.trim()||[t("Included manually"),t("Excluded manually")].includes(reason.value))reason.value=t(decision==="include"?"Included manually":decision==="exclude"?"Excluded manually":"Needs review");
      if(check.checked&&(!when.value||!when.checkValidity())){edit++;invoiceState.unsaved.add(index);status.className="error invoice-save-status";status.textContent=t("Enter a document date inside this period to save the correction.");invoiceStatus();return;}
      save({decision,invoice_date:when.value,kind:kind.value,reason:reason.value,setup_revision:invoiceState.preview.setup_revision,updated_at:0});
      invoiceStatus();
    };
    check.onchange=when.onchange=kind.onchange=reason.onchange=change;
    actions.append(node("label",{textContent:t("Include document")},check),node("label",{textContent:t("Verified document date")},when),node("label",{textContent:t("Document type")},kind));
    const reset=button(t("Use suggestion again"),()=>{if(![...invoiceState.unsaved].some(i=>i!==index))save(null,true);});
    const retry=button(t("Save correction"),()=>change({target:reason}));
    box.append(node("label",{textContent:t("Correction reason")},reason),status,node("div",{className:"card-actions"},retry,reset));
  }
  const mail=node("details",{},node("summary",{textContent:t("Read source mail without marking it read")}));
  const text=node("pre",{className:"invoice-mail"});mail.append(text);
  mail.addEventListener("toggle",async()=>{
    if(!mail.open || mail.dataset.loaded)return;mail.dataset.loaded="yes";text.textContent=t("Loading…");
    try { const detail=await invoke("message",{account:row.account,id:row.message});text.textContent=detail.body_text||t("No text available."); }
    catch(error){text.textContent=String(error);delete mail.dataset.loaded;}
  });
  box.append(actions,mail);return box;
}
function invoiceReceipt(result) {
  const box=node("section",{className:"invoice-receipt"});
  const count=status=>result.outcomes.filter(o=>o.status===status).length;
  if(result.finished===false)box.append(node("p",{className:"invoice-warning",textContent:t("This export was interrupted. Start a new preview to safely retry the remaining documents.")}));
  box.append(node("strong",{textContent:`${new Date(result.created_at*1000).toLocaleString()} · ${result.profile_name}`}),node("p",{textContent:t("{saved} saved · {duplicates} duplicates skipped · {failed} failed",{saved:count("saved"),duplicates:count("duplicate"),failed:count("failed")})}));
  if(result.manifest)box.append(node("p",{className:"hint",textContent:t("Export log: {path}",{path:filingPath(result.manifest)})}));
  if(result.manifest_error)box.append(node("p",{className:"error",textContent:t("The export log could not be written; the receipt remains in this profile's history.")+" "+result.manifest_error}));
  const details=node("details",{},node("summary",{textContent:t("Details")}));
  for(const o of result.outcomes)details.append(node("p",{textContent:`${o.name} · ${o.invoice_date} · ${t(o.status)} · ${filingPath(o.path)||o.error||""}`}));
  box.append(details);return box;
}
el("invoice-export").onclick=async()=>{
  const preview=invoiceState.preview,selection=[...invoiceState.selection.values()];
  if(!preview || invoiceState.busy || invoiceState.unsaved.size || !selection.length)return;
  const generation=invoiceState.generation;
  const folders=[...new Set(selection.map(s=>invoiceFolderForDate(preview,s.invoice_date)))];
  if(!await ask(t("Save {count} reviewed documents for profile {profile}?\n{path}\nThis copies attachments and writes an export log. Mail remains unchanged.",{count:selection.length,profile:preview.profile_name,path:folders.join("\n")})+"\n"+t("Missing date folders are created now. The export log stays in the selected base folder."),{yes:t("Confirm collection")}))return;
  if(generation!==invoiceState.generation)return;
  invoiceState.busy=true;el("invoice-close").disabled=true;invoiceStatus();
  try {
    const result=await invoke("confirm_invoice_collection",{token:preview.token,selection});
    if(generation!==invoiceState.generation)return;
    el("invoice-body").replaceChildren(invoiceReceipt(result),button(t("New preview / retry"),()=>openInvoiceCollection(preview.request.profile,{from:preview.request.from,to:preview.request.to})));
    invoiceState.preview=null;invoiceState.selection.clear();el("invoice-export").hidden=true;
    el("invoice-count").textContent=t("Collection finished. Review the receipt for failures.");
  }catch(error){say(String(error),true);}
  finally{invoiceState.busy=false;el("invoice-close").disabled=false;invoiceStatus();}
};
