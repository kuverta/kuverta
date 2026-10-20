// Friendly display names only. Folder ids and the original server names remain
// the values used for filtering, moving, creating and deleting mailboxes.
function folderPresentation(folder) {
  const name = folder.name;
  // INBOX. is the namespace used by many Dovecot servers. Do not split dots in
  // arbitrary user names such as "Invoices 2026.pdf".
  let parts = /^INBOX\./i.test(name) ? name.split('.') : name.split('/');
  if (parts.length > 1 && /^(INBOX|\[Gmail\]|\[Google Mail\])$/i.test(parts[0])) parts.shift();
  const names = {
    inbox: ['Inbox', 'inbox'],
    drafts: ['Drafts', 'drafts'],
    sent: ['Sent', 'sent'],
    'sent items': ['Sent', 'sent'],
    'sent messages': ['Sent', 'sent'],
    archive: ["Archive|a folder's name", 'archive'],
    archives: ['Archives', 'archive'],
    'all mail': ['All mail', 'archive'],
    junk: ['Spam', 'junk'],
    spam: ['Spam', 'junk'],
    trash: ['Trash', 'trash'],
    'deleted items': ['Trash', 'trash'],
    later: ['Later', 'folder'],
    blocked: ['Blocked', 'folder'],
  };
  const special = {
    '\\Drafts': ['Drafts', 'drafts'], '\\Sent': ['Sent', 'sent'],
    '\\Archive': ["Archive|a folder's name", 'archive'], '\\All': ['All mail', 'archive'],
    '\\Junk': ['Spam', 'junk'], '\\Trash': ['Trash', 'trash'],
  }[folder.special_use];
  let icon = 'folder';
  const labels = parts.map((part, at) => {
    const use = at === parts.length - 1 && special ? special : at === 0 ? names[part.toLowerCase()] : null;
    if (!use) return part;
    if (at === parts.length - 1) icon = use[1];
    return t(use[0]);
  });
  return { label: labels.join(' › '), depth: Math.max(0, parts.length - 1), icon };
}

function folderDisplayName(folder) {
  return folderPresentation(folder).label;
}
