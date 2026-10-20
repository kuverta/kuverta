// Browser-style locations, entirely separate from the mail operation/undo log.
// Kept in memory only: no message bodies or history survive closing the app.
const mailNavigation = {entries:[], at:-1, busy:false};
const navigationPreference = el("pref-mail-navigation");
try { navigationPreference.checked = localStorage.getItem("mailNavigation") === "yes"; } catch (_) {}
function navigationLocation() {
  const reader = !reading.hidden || !conversationPane.hidden || !el("mail-thread").hidden;
  const row = reader ? state.rows.get(state.selected) : null;
  return {profile:activeProfile(),account:state.account,postbox:state.postbox?.id??null,
    allInboxes:state.allInboxes,view:state.view,filter:{...state.filter},
    query:state.searching || state.view === "people" ? searchBox.value : "",
    order:mailOrder, message:row?.id??null, index:state.selected, scroll:viewport.scrollTop};
}
function navigationKey(place) {
  const {index,scroll,...location}=place; return JSON.stringify(location);
}
function navigationRemember() {
  if (!navigationPreference.checked || mailNavigation.busy || (!state.account && !state.allInboxes && !state.postbox)) return;
  const place = navigationLocation(), current = mailNavigation.entries[mailNavigation.at];
  if (current && navigationKey(current) === navigationKey(place)) { Object.assign(current,place); return; }
  mailNavigation.entries.splice(mailNavigation.at+1);
  mailNavigation.entries.push(place);
  if (mailNavigation.entries.length > 100) mailNavigation.entries.shift();
  mailNavigation.at = mailNavigation.entries.length-1;
  renderNavigation();
}
function renderNavigation() {
  el("mail-navigation").hidden = !navigationPreference.checked;
  el("mail-back").disabled = mailNavigation.busy || mailNavigation.at <= 0;
  el("mail-forward").disabled = mailNavigation.busy || mailNavigation.at >= mailNavigation.entries.length-1;
}
async function navigateMail(direction) {
  const at = mailNavigation.at + direction, place = mailNavigation.entries[at];
  if (!navigationPreference.checked || mailNavigation.busy || !place) return;
  if (!(await leaveCompose())) return;
  mailNavigation.busy = true;renderNavigation();
  try {
    if (place.profile !== null && !state.profiles.some(p=>p.id===place.profile)) throw Error(t("This navigation location is no longer available."));
    const account = state.accounts.find(a=>a.id===place.account);
    const postbox = state.postboxes.find(p=>p.id===place.postbox);
    if (!place.allInboxes && !account && !postbox) throw Error(t("This navigation location is no longer available."));
    if (place.profile !== null && !place.allInboxes && (postbox??account)?.profile_id !== place.profile) throw Error(t("This navigation location is no longer available."));
    setActiveProfile(place.profile);
    if (place.allInboxes) await selectAllInboxes();
    else if (postbox) await selectPostbox(postbox);
    else await selectAccount(account);
    state.view = place.view;state.filter = {...place.filter};mailOrder = place.order;
    searchBox.value = place.query;
    await reload();
    if (place.query && place.view !== "people") { searchBox.value = place.query; await runSearch(place.query); }
    if (place.message !== null) {
      let found = indexOfRow(place.message);
      const offset = Math.floor(Math.max(0,place.index)/PAGE)*PAGE;
      if (found < 0 && offset < state.total) { await loadPage(offset);found=indexOfRow(place.message); }
      // A sync may have moved the row. Only headers are paged, never bodies.
      for (let start=0;found<0 && start<state.total && !state.searching;start+=PAGE) {
        await loadPage(start);found=indexOfRow(place.message);
      }
      if (found >= 0) { select(found,{open:false});await openSelected({markRead:false}); }
      else say(t("This message is no longer in this view."));
    }
    viewport.scrollTop=place.scroll;
    mailNavigation.at=at;
  } catch(error) { say(String(error),true); }
  finally {mailNavigation.busy=false;renderNavigation();}
}
navigationPreference.onchange = () => {
  try {localStorage.setItem("mailNavigation",navigationPreference.checked?"yes":"no");} catch (_) {}
  mailNavigation.entries=[];mailNavigation.at=-1;navigationRemember();renderNavigation();
};
el("mail-back").onclick=()=>navigateMail(-1);
el("mail-forward").onclick=()=>navigateMail(1);
document.addEventListener("keydown",event=>{
  if (!navigationPreference.checked || !event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || !["ArrowLeft","ArrowRight"].includes(event.key)) return;
  if (event.target.closest("input,textarea,select,[contenteditable=true]") || document.querySelector(".sheet:not([hidden])")) return;
  event.preventDefault();event.stopImmediatePropagation();navigateMail(event.key==="ArrowLeft"?-1:1);
},true);
renderNavigation();

// These entry points are also reached from profile pills and keyboard search,
// outside the sidebar's navItem handler.
const navigationSwitchProfile = switchProfile;
switchProfile = async (...args) => { navigationRemember();await navigationSwitchProfile(...args);navigationRemember(); };
const navigationShowView = showView;
showView = async (...args) => { navigationRemember();await navigationShowView(...args);navigationRemember(); };
const navigationSearch = runSearch;
runSearch = async (...args) => { await navigationSearch(...args);navigationRemember(); };
