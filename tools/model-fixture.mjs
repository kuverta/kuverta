// Local-only model lists for manually testing Settings > Models without keys,
// downloaded models, real mail, or requests to a hosted service.
// node tools/model-fixture.mjs
import http from 'node:http';

const server = http.createServer(async (request, response) => {
  const path = new URL(request.url, 'http://127.0.0.1').pathname;
  let body = '';
  for await (const chunk of request) body += chunk;
  let result;
  if (request.method === 'GET' && path === '/ollama/api/tags') {
    result = { models: [
      { name: 'llama3.2:3b', size: 2019393189 },
      { name: 'qwen2.5vl:3b', size: 3200000000 },
    ] };
  } else if (request.method === 'POST' && path === '/ollama/api/show') {
    try {
      const { model } = JSON.parse(body);
      result = { capabilities: model === 'qwen2.5vl:3b' ? ['completion', 'vision'] : ['completion', 'tools'] };
    } catch {
      response.writeHead(400).end();
      return;
    }
  } else if (request.method === 'GET' && path === '/claude/v1/models') {
    result = { has_more: false, data: ['claude-haiku-4-5', 'claude-sonnet-4-5'].map((id) => ({
      id, capabilities: { image_input: { supported: true } },
    })) };
  } else {
    response.writeHead(501, { 'Content-Type': 'application/json' });
    response.end(JSON.stringify({ error: 'This fixture only lists models; it does not run them.' }));
    return;
  }
  response.writeHead(200, { 'Content-Type': 'application/json' });
  response.end(JSON.stringify(result));
});

server.listen(19325, '127.0.0.1', () => {
  console.log('Model fixture: Ollama http://127.0.0.1:19325/ollama; Claude http://127.0.0.1:19325/claude/v1');
});
