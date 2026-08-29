'use strict';

const AdmZip = require('adm-zip');
const fs = require('node:fs');
const http = require('node:http');
const https = require('node:https');
const os = require('node:os');
const path = require('node:path');
const tar = require('tar');

const {
  archiveNameForPlatform,
  artifactPrefix,
  detectPlatform,
  installedBinaryPath,
  releaseBaseUrlForVersion,
} = require('./platforms');
const { replaceFile } = require('./replace-binary');

async function bootstrapBinary(packageRoot, version, userAgent) {
  const platform = detectPlatform();
  const archiveName = archiveNameForPlatform(version, platform);
  const extension = archiveName.endsWith('.zip') ? 'zip' : 'tar.gz';
  const baseUrl = releaseBaseUrlForVersion(version);
  const archiveUrl = `${baseUrl}/${archiveName}`;
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), 'kalam-cli-bootstrap-'));

  try {
    console.log(`Bootstrapping ${archiveName}`);
    const archivePath = path.join(tempDir, archiveName);
    await downloadFile(archiveUrl, archivePath, userAgent);

    const extractDir = path.join(tempDir, 'extract');
    fs.mkdirSync(extractDir, { recursive: true });
    if (extension === 'zip') {
      new AdmZip(archivePath).extractAllTo(extractDir, true);
    } else {
      await tar.x({ file: archivePath, cwd: extractDir });
    }

    const binaryPath = findBinary(extractDir);
    if (!binaryPath) {
      throw new Error(`Could not find kalam binary in ${archiveName}`);
    }

    const distDir = path.join(packageRoot, 'dist');
    fs.mkdirSync(distDir, { recursive: true });
    replaceFile(binaryPath, installedBinaryPath(packageRoot));
  } finally {
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
}

function downloadFile(url, destination, userAgent) {
  return new Promise((resolve, reject) => {
    const client = url.startsWith('http://') ? http : https;
    const request = client.get(
      url,
      {
        headers: {
          'User-Agent': userAgent,
        },
      },
      (response) => {
        if ([301, 302, 303, 307, 308].includes(response.statusCode)) {
          response.resume();
          const location = response.headers.location;
          if (!location) {
            reject(new Error(`Redirect without location for ${url}`));
            return;
          }
          downloadFile(new URL(location, url).toString(), destination, userAgent).then(resolve, reject);
          return;
        }

        if (response.statusCode !== 200) {
          response.resume();
          reject(new Error(`Download failed for ${url} with HTTP ${response.statusCode}`));
          return;
        }

        const file = fs.createWriteStream(destination, { mode: 0o600 });
        response.pipe(file);
        file.on('finish', () => file.close(resolve));
        file.on('error', reject);
      }
    );

    request.on('error', reject);
    request.setTimeout(120000, () => {
      request.destroy(new Error(`Timed out downloading ${url}`));
    });
  });
}

function findBinary(root) {
  const stack = [root];
  while (stack.length > 0) {
    const current = stack.pop();
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const entryPath = path.join(current, entry.name);
      if (entry.isDirectory()) {
        stack.push(entryPath);
      } else if (
        ['kalam', 'kalam.exe', artifactPrefix].includes(entry.name) ||
        entry.name.startsWith(`${artifactPrefix}-`)
      ) {
        return entryPath;
      }
    }
  }
  return null;
}

module.exports = {
  bootstrapBinary,
};
