import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const exampleRoot = path.dirname(fileURLToPath(new URL('../package.json', import.meta.url)));
const reactDrizzle = path.resolve(
  exampleRoot,
  '../../link/sdks/typescript/react/node_modules/drizzle-orm',
);
const ormDrizzle = path.resolve(
  exampleRoot,
  '../../link/sdks/typescript/orm/node_modules/drizzle-orm',
);
const appDrizzle = path.join(exampleRoot, 'node_modules/drizzle-orm');

if (!fs.existsSync(path.join(reactDrizzle, 'package.json'))) {
  console.error(
    'Install link/sdks/typescript/react before this example so drizzle-orm is one module.',
  );
  process.exit(1);
}

const source = fs.realpathSync(reactDrizzle);

function linkOver(target) {
  if (fs.existsSync(target)) {
    const stat = fs.lstatSync(target);
    if (stat.isSymbolicLink() && fs.realpathSync(target) === source) {
      return;
    }
    fs.rmSync(target, { recursive: true, force: true });
  }
  fs.symlinkSync(source, target, 'dir');
}

linkOver(appDrizzle);
if (fs.existsSync(path.dirname(ormDrizzle))) {
  linkOver(ormDrizzle);
}
