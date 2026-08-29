'use strict';

const { spawn } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const { installedBinaryPath } = require('./platforms');

const REPLACE_RETRY_ATTEMPTS = 10;

function replaceFile(source, destination) {
  fs.mkdirSync(path.dirname(destination), { recursive: true });
  const stalePath = `${destination}.kalam-old`;
  removeIfPresent(stalePath);

  if (fs.existsSync(destination)) {
    retry(() => fs.renameSync(destination, stalePath), destination);
  }

  try {
    fs.copyFileSync(source, destination);
  } catch (error) {
    if (fs.existsSync(stalePath)) {
      try {
        fs.renameSync(stalePath, destination);
      } catch {
        // Keep the aside copy if rollback also fails.
      }
    }
    throw error;
  }
  if (process.platform !== 'win32') {
    fs.chmodSync(destination, 0o755);
  }

  if (!removeIfPresent(stalePath) && process.platform === 'win32') {
    scheduleDelete(stalePath);
  }
}

function unlockInstalledBinary(packageRoot) {
  const binaryPath = installedBinaryPath(packageRoot);
  if (!fs.existsSync(binaryPath)) {
    return null;
  }

  const unlockedPath = path.join(os.tmpdir(), `kalam-unlocked-${process.pid}.exe`);
  removeIfPresent(unlockedPath);
  retry(() => fs.renameSync(binaryPath, unlockedPath), binaryPath);
  if (process.platform === 'win32') {
    scheduleDelete(unlockedPath);
  }
  return unlockedPath;
}

function removeIfPresent(targetPath) {
  try {
    fs.rmSync(targetPath, { force: true });
    return true;
  } catch {
    return false;
  }
}

function retry(op, lockedPath) {
  let lastError;
  for (let attempt = 0; attempt < REPLACE_RETRY_ATTEMPTS; attempt += 1) {
    try {
      op();
      return;
    } catch (error) {
      lastError = error;
      sleepSync(50 * (attempt + 1));
    }
  }

  throw new Error(
    `Failed to replace ${lockedPath}: ${lastError?.message ?? 'file is in use'}. Close other kalam processes and retry`
  );
}

function scheduleDelete(targetPath) {
  const command = `ping -n 3 127.0.0.1 >nul & del /F /Q ${quoteCmdPath(targetPath)}`;
  spawn('cmd.exe', ['/C', command], {
    detached: true,
    stdio: 'ignore',
    windowsHide: true,
    windowsVerbatimArguments: true,
  }).unref();
}

function quoteCmdPath(targetPath) {
  return `"${String(targetPath).replaceAll('"', '""')}"`;
}

function sleepSync(ms) {
  const lock = new Int32Array(new SharedArrayBuffer(4));
  Atomics.wait(lock, 0, 0, ms);
}

module.exports = {
  quoteCmdPath,
  replaceFile,
  unlockInstalledBinary,
};
