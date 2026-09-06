#!/usr/bin/env node
'use strict';

const { packageRootFromEnv } = require('./platforms');
const { unlockInstalledBinary } = require('./replace-binary');

if (process.platform !== 'win32') {
  process.exit(0);
}

try {
  unlockInstalledBinary(packageRootFromEnv());
} catch (error) {
  // Best-effort: npm may still fail to remove a locked kalam.exe.
  console.warn(`Could not move kalam.exe aside before uninstall: ${error.message}`);
}
