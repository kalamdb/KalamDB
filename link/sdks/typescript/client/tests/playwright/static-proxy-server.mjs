import http from 'node:http';
import https from 'node:https';
import fs from 'node:fs';
import net from 'node:net';
import path from 'node:path';
import tls from 'node:tls';
import { fileURLToPath } from 'node:url';

const port = Number(process.env.PORT ?? 41731);
const staticRoot = process.env.STATIC_ROOT ?? process.cwd();
const backendUrl = new URL(process.env.BACKEND_URL ?? 'http://127.0.0.1:2900');
const wsPairs = new Set();

const contentTypes = new Map([
  ['.html', 'text/html; charset=utf-8'],
  ['.js', 'text/javascript; charset=utf-8'],
  ['.mjs', 'text/javascript; charset=utf-8'],
  ['.css', 'text/css; charset=utf-8'],
  ['.json', 'application/json; charset=utf-8'],
  ['.wasm', 'application/wasm'],
  ['.map', 'application/json; charset=utf-8'],
]);

function resolveStaticPath(urlPath) {
  const pathname = urlPath === '/' ? '/tests/browser-apollo-e2e.html' : urlPath;
  const safePath = path.normalize(pathname).replace(/^([.][.][/\\])+/, '');
  const fullPath = path.resolve(staticRoot, `.${safePath}`);
  if (!fullPath.startsWith(path.resolve(staticRoot))) {
    return null;
  }
  return fullPath;
}

function proxyPath(requestPath) {
  const stripped = requestPath.replace(/^\/backend/, '') || '/';
  return stripped.startsWith('/') ? stripped : `/${stripped}`;
}

function backendPort() {
  return Number(backendUrl.port || (backendUrl.protocol === 'https:' ? 443 : 80));
}

function proxyHttp(req, res) {
  const target = new URL(proxyPath(req.url ?? '/'), backendUrl);
  const transport = target.protocol === 'https:' ? https : http;
  const upstream = transport.request(
    {
      protocol: target.protocol,
      hostname: target.hostname,
      port: target.port,
      method: req.method,
      path: `${target.pathname}${target.search}`,
      headers: {
        ...req.headers,
        host: target.host,
      },
    },
    (upstreamRes) => {
      res.writeHead(upstreamRes.statusCode ?? 502, upstreamRes.headers);
      upstreamRes.pipe(res);
    },
  );

  upstream.on('error', (error) => {
    res.writeHead(502, { 'content-type': 'text/plain; charset=utf-8' });
    res.end(`Proxy error: ${error.message}`);
  });

  req.pipe(upstream);
}

function dropActiveWebSockets(res) {
  for (const pair of wsPairs) {
    pair.client.destroy();
    pair.upstream.destroy();
  }
  wsPairs.clear();
  res.writeHead(204);
  res.end();
}

function serveStatic(req, res) {
  const filePath = resolveStaticPath(new URL(req.url ?? '/', 'http://local').pathname);
  if (!filePath) {
    res.writeHead(403, { 'content-type': 'text/plain; charset=utf-8' });
    res.end('Forbidden');
    return;
  }

  fs.stat(filePath, (err, stat) => {
    if (err || !stat.isFile()) {
      res.writeHead(404, { 'content-type': 'text/plain; charset=utf-8' });
      res.end('Not found');
      return;
    }

    const ext = path.extname(filePath);
    res.writeHead(200, {
      'content-type': contentTypes.get(ext) ?? 'application/octet-stream',
      'cache-control': 'no-store',
    });
    fs.createReadStream(filePath).pipe(res);
  });
}

const server = http.createServer((req, res) => {
  if ((req.url ?? '') === '/__drop_ws' && req.method === 'POST') {
    dropActiveWebSockets(res);
    return;
  }

  if ((req.url ?? '').startsWith('/backend')) {
    proxyHttp(req, res);
    return;
  }
  serveStatic(req, res);
});

server.on('upgrade', (req, socket, head) => {
  if (!(req.url ?? '').startsWith('/backend')) {
    socket.destroy();
    return;
  }

  const upstream = backendUrl.protocol === 'https:'
    ? tls.connect({
      host: backendUrl.hostname,
      port: backendPort(),
      servername: backendUrl.hostname,
    })
    : net.connect({
      host: backendUrl.hostname,
      port: backendPort(),
    });

  upstream.on('connect', () => {
    const lines = [`GET ${proxyPath(req.url ?? '/')} HTTP/1.1`];
    for (const [key, value] of Object.entries(req.headers)) {
      if (value === undefined) {
        continue;
      }
      if (key === 'host' || key === 'origin') {
        continue;
      }
      lines.push(`${key}: ${Array.isArray(value) ? value.join(', ') : value}`);
    }
    lines.push(`host: ${backendUrl.host}`);
    lines.push('', '');
    upstream.write(lines.join('\r\n'));
    if (head.length > 0) {
      upstream.write(head);
    }

    socket.pipe(upstream).pipe(socket);
  });

  const closeBoth = () => {
    socket.destroy();
    upstream.destroy();
  };

  const pair = { client: socket, upstream };
  wsPairs.add(pair);

  const forgetPair = () => {
    wsPairs.delete(pair);
  };

  upstream.on('error', closeBoth);
  socket.on('error', closeBoth);
  socket.on('close', () => {
    forgetPair();
    upstream.end();
  });
  upstream.on('close', () => {
    forgetPair();
    socket.end();
  });
});

server.listen(port, '127.0.0.1', () => {
  const rootLabel = path.relative(process.cwd(), staticRoot) || '.';
  const thisFile = fileURLToPath(import.meta.url);
  console.log(`Browser test server ready on http://127.0.0.1:${port} (static=${rootLabel}, backend=${backendUrl.href}, runner=${path.basename(thisFile)})`);
});