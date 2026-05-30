#!/usr/bin/env node

import fs from 'node:fs';
import path from 'node:path';

const [sdkDirArg, stageDirArg, sourceScope, targetScope] = process.argv.slice(2);

if (!sdkDirArg || !stageDirArg || !sourceScope || !targetScope) {
  throw new Error(
    'Usage: node prepare-publish-dir.mjs <sdk-dir> <stage-dir> <source-scope> <target-scope>',
  );
}

const sdkDir = path.resolve(process.cwd(), sdkDirArg);
const stageDir = path.resolve(process.cwd(), stageDirArg);
const packageJsonPath = path.join(sdkDir, 'package.json');

if (!fs.existsSync(packageJsonPath)) {
  throw new Error(`Could not find package.json at ${packageJsonPath}`);
}

const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, 'utf8'));
const publishRegistryUrl = (process.env.PUBLISH_REGISTRY_URL || 'https://registry.npmjs.org').replace(/\/$/, '');

const escapeRegex = (value) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

const rewritePackageName = (value) => {
  if (typeof value !== 'string') {
    return value;
  }

  if (!value.startsWith(`${sourceScope}/`)) {
    return value;
  }

  return `${targetScope}/${value.slice(sourceScope.length + 1)}`;
};

const rewriteDependencyValue = (value) => {
  if (typeof value !== 'string') {
    return value;
  }

  const aliasPattern = new RegExp(`^npm:${escapeRegex(sourceScope)}/`);
  if (aliasPattern.test(value)) {
    return value.replace(aliasPattern, `npm:${targetScope}/`);
  }

  return value;
};

const rewriteDependencyMap = (section) => {
  if (!section || typeof section !== 'object') {
    return section;
  }

  return Object.fromEntries(
    Object.entries(section).map(([key, value]) => [rewritePackageName(key), rewriteDependencyValue(value)]),
  );
};

const rewriteMetadataMap = (section) => {
  if (!section || typeof section !== 'object') {
    return section;
  }

  return Object.fromEntries(Object.entries(section).map(([key, value]) => [rewritePackageName(key), value]));
};

const rewriteText = (value) => {
  if (typeof value !== 'string') {
    return value;
  }

  return value.replaceAll(`${sourceScope}/`, `${targetScope}/`);
};

const stagedPackageJson = {
  ...packageJson,
  name: rewritePackageName(packageJson.name),
  dependencies: rewriteDependencyMap(packageJson.dependencies),
  devDependencies: rewriteDependencyMap(packageJson.devDependencies),
  optionalDependencies: rewriteDependencyMap(packageJson.optionalDependencies),
  peerDependencies: rewriteDependencyMap(packageJson.peerDependencies),
  peerDependenciesMeta: rewriteMetadataMap(packageJson.peerDependenciesMeta),
};

if (typeof stagedPackageJson.repository === 'string') {
  if (stagedPackageJson.repository.startsWith('https://github.com/')) {
    stagedPackageJson.repository = `git+${stagedPackageJson.repository.replace(/\.git$/, '')}.git`;
  }
} else if (
  stagedPackageJson.repository &&
  typeof stagedPackageJson.repository === 'object' &&
  typeof stagedPackageJson.repository.url === 'string' &&
  stagedPackageJson.repository.url.startsWith('https://github.com/')
) {
  stagedPackageJson.repository = {
    ...stagedPackageJson.repository,
    url: `git+${stagedPackageJson.repository.url.replace(/\.git$/, '')}.git`,
  };
}

if (publishRegistryUrl !== 'https://registry.npmjs.org' && stagedPackageJson.publishConfig) {
  const publishConfig = { ...stagedPackageJson.publishConfig };
  delete publishConfig.access;

  if (Object.keys(publishConfig).length === 0) {
    delete stagedPackageJson.publishConfig;
  } else {
    stagedPackageJson.publishConfig = publishConfig;
  }
}

fs.rmSync(stageDir, { recursive: true, force: true });
fs.mkdirSync(stageDir, { recursive: true });

const copyEntry = (name) => {
  const sourcePath = path.join(sdkDir, name);
  if (!fs.existsSync(sourcePath)) {
    return;
  }

  fs.cpSync(sourcePath, path.join(stageDir, name), { recursive: true });
};

copyEntry('dist');

for (const entry of fs.readdirSync(sdkDir)) {
  if (
    /^README(?:\..+)?$/i.test(entry) ||
    /^LICEN[CS]E(?:\..+)?$/i.test(entry) ||
    /^NOTICE(?:\..+)?$/i.test(entry)
  ) {
    const sourcePath = path.join(sdkDir, entry);
    if (fs.statSync(sourcePath).isFile()) {
      let contents = fs.readFileSync(sourcePath, 'utf8');
      if (/^README(?:\..+)?$/i.test(entry)) {
        contents = rewriteText(contents);
      }
      fs.writeFileSync(path.join(stageDir, entry), contents, 'utf8');
    }
  }
}

fs.writeFileSync(path.join(stageDir, 'package.json'), `${JSON.stringify(stagedPackageJson, null, 2)}\n`, 'utf8');