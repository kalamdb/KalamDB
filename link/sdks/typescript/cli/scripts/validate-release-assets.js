#!/usr/bin/env node
'use strict';

const http = require('node:http');
const https = require('node:https');
const path = require('node:path');
const { archiveNameForPlatform, releaseBaseUrlForVersion, supportedPlatforms } = require('./install.js');

const packageRoot = path.resolve(__dirname, '..');
const packageJson = require(path.join(packageRoot, 'package.json'));
const version = (readOption('--version') || process.env.KALAM_CLI_VERSION || packageJson.version).replace(/^v/, '');
const baseUrl = (readOption('--base-url') || releaseBaseUrlForVersion(version)).replace(/\/+$/, '');

main().catch((error) => {
  console.error(`Failed to validate KalamDB CLI release assets: ${error.message}`);
  process.exit(1);
});

async function main() {
  const checksumsUrl = `${baseUrl}/SHA256SUMS`;
  const checksums = parseChecksums(await fetchText(checksumsUrl));
  const errors = [];

  for (const platform of supportedPlatforms) {
    const archiveName = archiveNameForPlatform(version, platform.name);
    const checksum = checksums.get(archiveName);
    if (!checksum) {
      errors.push(`SHA256SUMS does not include ${archiveName}`);
    } else if (!/^[a-f0-9]{64}$/i.test(checksum)) {
      errors.push(`SHA256SUMS has an invalid checksum for ${archiveName}`);
    }

    const archiveUrl = `${baseUrl}/${archiveName}`;
    try {
      const statusCode = await assertUrlExists(archiveUrl);
      console.log(`Validated ${archiveName} (HTTP ${statusCode})`);
    } catch (error) {
      errors.push(error.message);
    }
  }

  if (errors.length > 0) {
    throw new Error(`\n${errors.map((error) => `- ${error}`).join('\n')}`);
  }

  console.log(`Validated ${supportedPlatforms.length} KalamDB CLI release artifacts for ${version}`);
}

function readOption(name) {
  const index = process.argv.indexOf(name);
  if (index === -1) {
    return '';
  }

  const value = process.argv[index + 1];
  if (!value || value.startsWith('--')) {
    throw new Error(`${name} requires a value`);
  }

  return value;
}

function parseChecksums(body) {
  const entries = new Map();
  for (const line of body.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed) {
      continue;
    }

    const parts = trimmed.split(/\s+/);
    if (parts.length >= 2) {
      entries.set(normalizeChecksumName(parts[1]), parts[0]);
    }
  }
  return entries;
}

function normalizeChecksumName(name) {
  return name.replace(/^\*/, '').replace(/^\.\//, '');
}

async function assertUrlExists(url) {
  const response = await request(url, { method: 'HEAD' });
  if (response.statusCode >= 200 && response.statusCode < 300) {
    return response.statusCode;
  }

  if (response.statusCode === 405) {
    const fallback = await request(url, { method: 'GET', headers: { Range: 'bytes=0-0' } });
    if ((fallback.statusCode >= 200 && fallback.statusCode < 300) || fallback.statusCode === 206) {
      return fallback.statusCode;
    }
  }

  throw new Error(`Download link failed for ${url} with HTTP ${response.statusCode}`);
}

async function fetchText(url) {
  const response = await request(url, { method: 'GET', collectBody: true });
  if (response.statusCode !== 200) {
    throw new Error(`Download failed for ${url} with HTTP ${response.statusCode}`);
  }
  return response.body;
}

function request(url, options = {}, redirectsRemaining = 5) {
  return new Promise((resolve, reject) => {
    const parsed = new URL(url);
    const client = parsed.protocol === 'http:' ? http : https;
    const requestOptions = {
      method: options.method || 'GET',
      headers: {
        'User-Agent': `@kalamdb/cli/${packageJson.version}`,
        ...(options.headers || {}),
      },
    };

    const req = client.request(parsed, requestOptions, (res) => {
      if ([301, 302, 303, 307, 308].includes(res.statusCode)) {
        res.resume();
        const location = res.headers.location;
        if (!location) {
          reject(new Error(`Redirect without location for ${url}`));
          return;
        }
        if (redirectsRemaining <= 0) {
          reject(new Error(`Too many redirects for ${url}`));
          return;
        }
        request(new URL(location, url).toString(), options, redirectsRemaining - 1).then(resolve, reject);
        return;
      }

      if (!options.collectBody) {
        res.resume();
        resolve({ statusCode: res.statusCode });
        return;
      }

      res.setEncoding('utf8');
      let body = '';
      res.on('data', (chunk) => {
        body += chunk;
      });
      res.on('end', () => resolve({ statusCode: res.statusCode, body }));
    });

    req.on('error', reject);
    req.setTimeout(120000, () => {
      req.destroy(new Error(`Timed out requesting ${url}`));
    });
    req.end();
  });
}