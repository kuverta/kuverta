// Interactive browser preview of the real desktop UI with the same fake RPC
// bridge used by the browser tests. No Rust build or external accounts needed.
import http from 'node:http';
import { readFile } from 'node:fs/promises';
import { dirname, extname, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const allowed = ['apps/desktop/ui', 'kuverta-bird/test/support', 'kuverta-bird/core', 'kuverta-bird/hosts/kuverta']
  .map((path) => resolve(root, path) + sep);
const types = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css', '.svg': 'image/svg+xml', '.png': 'image/png' };
const bridge = `<script>
localStorage.setItem('language', 'de');
const previewBridge = import('/kuverta-bird/test/support/fake-invoke.js').then(async ({fakeInvoke}) => {
  const invoke = fakeInvoke({paper: {paperMailboxes: [], documents: []}});
  await invoke('save_ai_provider', {input: {id: null, kind: 'anthropic', label: 'Claude (Test)', base_url: 'https://example.invalid'}});
  return invoke;
});
window.__TAURI__ = {core: {
  invoke: async (command, args) => {
    if (command === 'ai_models' && args.providerId === 2) return ['claude-haiku-4-5', 'claude-sonnet-4-5'].map(name => ({name, vision: true, embedding: false, size_bytes: null}));
    return (await previewBridge)(command, args);
  },
  Channel: class { onmessage = null; }
}};
</script>`;

http.createServer(async (request, response) => {
  try {
    const path = decodeURIComponent(new URL(request.url, 'http://127.0.0.1').pathname);
    if (path === '/') {
      response.writeHead(302, { Location: '/apps/desktop/ui/index.html' }).end();
      return;
    }
    const file = resolve(root, '.' + path);
    if (!allowed.some((prefix) => file.startsWith(prefix))) {
      response.writeHead(403).end();
      return;
    }
    let body = await readFile(file);
    if (path === '/apps/desktop/ui/index.html') {
      body = body.toString().replace('<script src="i18n.js">', bridge + '<script src="i18n.js">')
        .replace('<title>kuverta</title>', '<title>Kuverta – lokale Testvorschau #25</title>')
        .replace('<body>', '<body><div style="position:fixed;bottom:0;left:0;right:0;z-index:99999;background:#fff4c2;color:#402c00;padding:6px;text-align:center;font:13px system-ui">Testvorschau #25 · Beispieldaten · keine echten E-Mails oder Modellaufrufe · Einstellungen → Modell für jede Aufgabe</div>');
    }
    response.writeHead(200, { 'Content-Type': types[extname(file)] ?? 'application/octet-stream', 'Cache-Control': 'no-store' });
    response.end(body);
  } catch {
    response.writeHead(404).end();
  }
}).listen(19326, '127.0.0.1', () => console.log('Kuverta model preview: http://127.0.0.1:19326'));
