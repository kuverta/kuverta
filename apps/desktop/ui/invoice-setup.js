// The setup never reads documents or files them. Only its final step confirms
// the profile policy; intermediate saves preserve an unfinished draft.
function invoiceFolderPicker(input) {
  const choose=button(t("Choose folder…"),async()=>{
    choose.disabled=true;
    try {
      const path=await invoke("pick_invoice_folder",{initial:input.value});
      if(path&&input.isConnected){input.value=filingPath(path);input.dispatchEvent(new Event("input",{bubbles:true}));}
    }catch(error){say(String(error),true);}
    finally{choose.disabled=false;}
  });
  choose.className="invoice-folder-picker";
  // Other platforms keep the editable path field; no shell commands are used.
  choose.hidden=!navigator.platform.startsWith("Win");
  return choose;
}
function invoiceLayoutLabel(layout) {return t(layout==="year_quarter_month"?"Year → quarter → month (2026/Q3/09)":"Directly in the selected folder");}
function invoiceFolderForDate(preview,when) {
  const root=filingPath(preview.request.root).replace(/[\\/]$/,""),separator=root.includes("\\")?"\\":"/";
  if(preview.layout!=="year_quarter_month")return root;
  if(!/^\d{4}-\d{2}-\d{2}$/.test(when))return t("Choose a verified document date to see its destination.");
  return [root,when.slice(0,4),`Q${Math.ceil(Number(when.slice(5,7))/3)}`,when.slice(5,7)].join(separator);
}
function invoicePeriodBounds(period,now=new Date()) {
  const year=now.getFullYear(),month=now.getMonth();
  if(period==="month")return [new Date(year,month-1,1),new Date(year,month,0)];
  if(period==="year")return [new Date(year-1,0,1),new Date(year-1,11,31)];
  return [new Date(year,Math.floor(month/3)*3-3,1),new Date(year,Math.floor(month/3)*3,0)];
}
function invoicePeriodOptions(select) {
  for(const [value,label] of [["month","Previous month"],["quarter","Previous quarter"],["year","Previous year"],["custom","Custom dates"]])select.append(new Option(t(label),value));
}
async function renderInvoiceSetup(profile,settings,options={}) {
  const saved=settings.setup??{},generation=invoiceState.generation;
  const config=structuredClone(saved.draft??saved.confirmed??{
    company:"",aliases:[],address:"",people:[],incoming:true,outgoing:false,credits:true,
    review_terms:["Testrechnung","test invoice","sample invoice","Musterrechnung","pro forma","proforma","privat","private"],
    accounts:[],root:filingPath(settings.root),layout:"year_quarter_month",period:"month",custom_from:"",custom_to:""
  });
  config.root=filingPath(config.root);
  config.layout??="flat";
  let step=0,saving=false,sourceConfirmed=false;
  const body=el("invoice-body"),foot=el("invoice-count");
  el("invoice-title").textContent=`${t("Set up invoice collection")} · ${profile.name}`;
  const save=async(confirm=false)=>{
    if(saving)return false;saving=true;
    body.querySelectorAll("button").forEach(b=>b.disabled=true);el("invoice-close").disabled=true;
    foot.textContent=t("Saving…");
    try {
      const result=await invoke("save_invoice_setup",{profile:profile.id,setup:config,expectedRevision:saved.revision??0,confirm});
      if(generation!==invoiceState.generation)return false;
      settings.setup=result;foot.textContent=t(confirm?"Setup saved":"Setup draft saved");return true;
    }catch(error){foot.textContent=t("Could not save. Your changes are still open.");say(String(error),true);return false;}
    finally{saving=false;body.querySelectorAll("button").forEach(b=>b.disabled=false);el("invoice-close").disabled=false;}
  };
  invoiceState.wizardClose=async()=>{if(await save())closeDialog(invoiceSheet);};
  el("invoice-close").textContent=t("Save draft and close");el("invoice-close").onclick=invoiceState.wizardClose;
  const field=(key,label,{list=false,required=false,type="text"}={})=>{
    const input=node(list||key==="address"?"textarea":"input",{id:`invoice-setup-${key}`,required});
    if(input.tagName==="INPUT")input.type=type;
    input.value=list?config[key].join("\n"):config[key];
    input.oninput=()=>{config[key]=list?input.value.split("\n").map(s=>s.trim()).filter(Boolean):input.value;};
    return node("label",{textContent:t(label)},input);
  };
  const check=(key,label)=>{
    const input=node("input",{type:"checkbox",id:`invoice-setup-${key}`,checked:config[key]});input.onchange=()=>{config[key]=input.checked;};
    return node("label",{className:"check",textContent:t(label)},input);
  };
  const draw=()=>{
    body.replaceChildren();
    const titles=["Company and people","Documents to collect","Mail accounts","Destination and period","Review setup"];
    body.append(node("p",{className:"hint",textContent:t("Step {step} of {total}",{step:step+1,total:5})}),node("h3",{textContent:t(titles[step])}));
    const form=node("form",{id:"invoice-setup-form"});body.append(form);
    if(step===0)form.append(field("company","Actual company / invoice recipient",{required:true}),field("aliases","Other company spellings (one per line)",{list:true}),field("address","Company address (optional; a missing match requires review)"),field("people","Associated people (one per line; always review personal invoices)",{list:true}),node("p",{className:"hint",textContent:t("A profile name or a person's name alone does not prove that a document belongs to the company.")}));
    if(step===1)form.append(check("incoming","Incoming invoices"),check("outgoing","Outgoing invoices"),check("credits","Include credit notes"),field("review_terms","Always review these terms (one per line)",{list:true}),node("p",{className:"hint",textContent:t("These terms prevent automatic selection. A manual correction applies only to that exact document in this profile, not every invoice from its sender.")}));
    if(step===2){
      for(const id of profile.accounts)form.append(node("p",{textContent:state.accounts.find(a=>a.id===id)?.email??String(id)}));
      form.append(node("p",{className:"hint",textContent:t("All downloaded mail in these accounts is searched, including sent mail, spam and trash. Mail account and invoice recipient are checked separately.")}));
      const ack=node("input",{id:"invoice-setup-accounts",type:"checkbox",required:true,checked:sourceConfirmed});ack.onchange=()=>{sourceConfirmed=ack.checked;if(ack.checked)config.accounts=[...profile.accounts];};
      form.append(node("label",{className:"check",textContent:t("Use all these accounts for this profile")},ack));
    }
    if(step===3){
      const root=field("root","Profile invoice destination (existing folder)",{required:true});
      root.append(invoiceFolderPicker(root.querySelector("input")));form.append(root);
      const layout=node("select",{id:"invoice-setup-layout"});
      for(const value of ["year_quarter_month","flat"])layout.append(new Option(invoiceLayoutLabel(value),value));
      layout.value=config.layout;layout.onchange=()=>{config.layout=layout.value;};
      form.append(node("label",{textContent:t("Folder structure")},layout),node("p",{className:"hint",textContent:t("Uses the verified invoice date. Existing folders with these names are reused; missing folders are created only after collection approval. Other folders and files are not reorganized or overwritten.")}));
      const period=node("select",{id:"invoice-setup-period"});invoicePeriodOptions(period);period.value=config.period;
      const dates=node("div",{className:"invoice-fields"},field("custom_from","Default invoice date from",{type:"date"}),field("custom_to","Default invoice date to",{type:"date"}));
      const update=()=>{config.period=period.value;dates.hidden=period.value!=="custom";dates.querySelectorAll("input").forEach(i=>i.required=!dates.hidden);};period.onchange=update;update();
      form.append(node("label",{textContent:t("Default period for new collections")},period),dates,node("p",{className:"hint",textContent:t("Every collection can use different start and end dates, including periods across years. Saving copies does not send them to DATEV.")}));
    }
    if(step===4){
      const summary=node("dl",{className:"invoice-setup-summary"});
      for(const [label,value] of [["Company and people",[config.company,...config.aliases,config.address,...config.people].filter(Boolean).join(" · ")],["Documents to collect",[config.incoming?t("Incoming invoices"):null,config.outgoing?t("Outgoing invoices"):null,config.credits?t("Include credit notes"):null].filter(Boolean).join(" · ")],["Always review these terms (one per line)",config.review_terms.join(" · ")],["Mail accounts",profile.accounts.map(id=>state.accounts.find(a=>a.id===id)?.email??id).join(" · ")],["Profile invoice destination (existing folder)",config.root],["Default period for new collections",config.period==="custom"?`${config.custom_from} – ${config.custom_to}`:t(({month:"Previous month",quarter:"Previous quarter",year:"Previous year"})[config.period])]])summary.append(node("dt",{textContent:t(label)}),node("dd",{textContent:value||"—"}));
      summary.append(node("dt",{textContent:t("Folder structure")}),node("dd",{textContent:invoiceLayoutLabel(config.layout)}));
      form.append(summary,node("p",{className:"hint",textContent:t("Confirming setup does not read or save documents. Each collection still needs its own preview and approval. Changed settings require earlier corrections to be reviewed again.")}));
    }
    const actions=node("div",{className:"card-actions"});
    if(step)actions.append(button(t("Back"),async()=>{if(await save()){step--;draw();}}));
    actions.append(node("button",{type:"submit",className:"primary",textContent:t(step===4?"Confirm setup":"Continue")}));form.append(actions);
    form.onsubmit=async event=>{event.preventDefault();if(saving)return;
      if(step===1&&!config.incoming&&!config.outgoing){say(t("Choose incoming or outgoing invoices."),true);return;}
      if(step===3&&config.period==="custom"&&(!config.custom_from||!config.custom_to||config.custom_to<config.custom_from)){say(t("Invalid invoice period. Request a new preview."),true);return;}
      if(await save(step===4)){if(step===4){invoiceState.wizardClose=null;await openInvoiceCollection(profile.id,{...options,setup:false,autoStart:false});}else{step++;draw();}}
    };
  };
  draw();
}
