// Publication diagnostics never save the account, send mail or edit its DNS.
// Cache only this window's results, briefly; imported accounts may share a domain.
const domainChecks = new Map();
function mailDomain(address) { return address.trim().split('@').slice(1).join('@'); }

function makeDomainCheck(address, automatic = false) {
  const domain = mailDomain(address);
  const box = node('section', {className:'domain-check'});
  box.append(node('h3', {}, t('Mail domain check')),
    node('p', {className:'hint'}, t('Checks public DNS using Google Public DNS. Only the domain and optional DKIM selector are sent, not your address, password or mail.')));
  const selector = node('input', {type:'text', placeholder:t('Optional, e.g. selector1'), spellcheck:false});
  const result = node('div', {className:'domain-check-results', role:'status', ariaLive:'polite'});
  const run = button(t('Check domain'), () => check(true));
  box.append(node('label', {}, t('DKIM selector'), selector), run, result,
    node('p', {className:'hint'}, t('This is a DNS publication check. It does not test delivery, sending IPs, DKIM signatures or DMARC alignment. Mail setup can continue independently.')));
  let ticket = 0;
  selector.addEventListener('input', () => {
    ticket += 1;
    result.replaceChildren(node('p', {className:'hint'}, t('Selector changed. Run the check again.')));
    run.disabled = false;
  });
  async function check(fresh = false) {
    const current = ++ticket;
    const key = `${domain.toLowerCase()}|${selector.value.trim()}`;
    if (!domain || domain.includes('@')) {
      result.textContent = t('Enter a valid email address first.');
      return;
    }
    run.disabled = true;
    result.textContent = t('Checking domain…');
    try {
      const cached = domainChecks.get(key);
      let pending = !fresh && cached && Date.now() - cached.at < 300000 ? cached.pending : null;
      if (!pending) {
        pending = invoke('check_mail_domain', {domain, selector:selector.value.trim() || null});
        if (domainChecks.size >= 64) domainChecks.delete(domainChecks.keys().next().value);
        domainChecks.set(key, {at:Date.now(), pending});
      }
      const report = await pending;
      if (current !== ticket) return;
      result.replaceChildren(node('p', {className:'domain-checked-name'}, report.domain));
      const labels = {present:'Record found', warning:'Review', error:'Configuration issue', missing:'No record found', unavailable:'Lookup unavailable', not_checked:'Not checked'};
      for (const item of report.checks) {
        const entry = node('div', {className:`domain-check-item ${item.status}`});
        entry.append(node('strong', {}, `${item.protocol} · ${t(labels[item.status] ?? 'Not checked')}`));
        for (const note of item.notes) entry.append(node('p', {}, t(note)));
        if (item.name) entry.append(node('code', {}, item.name));
        if (item.records.length) {
          entry.append(node('details', {}, node('summary', {}, t('Show DNS records')),
            ...item.records.map(record => node('pre', {}, record))));
        }
        result.append(entry);
      }
    } catch (error) {
      domainChecks.delete(key);
      if (current === ticket) result.textContent = `${t('Lookup unavailable')}: ${t(String(error))}`;
    } finally {
      if (current === ticket) run.disabled = false;
    }
  }
  if (automatic && domain) queueMicrotask(() => check());
  return box;
}

function resetAccountDomainCheck() {
  const mount = el('account-domain-check');
  if (!mount) return;
  const email = settings.form.email.value;
  // Existing accounts are checked on demand. A new address starts the check.
  mount.replaceChildren(makeDomainCheck(email, settings.editing === null && !!email));
}
settings.form.email.addEventListener('change', resetAccountDomainCheck);
