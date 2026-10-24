#!/usr/bin/env python3
"""Compile real generated TS/Dart output against installed local SDK dependencies."""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[3]
FIXTURES = Path(__file__).resolve().parent / 'fixtures'
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--flutter', default='flutter')
parser.add_argument('--kalam', default=str(ROOT / 'target/debug/kalam'))
parser.add_argument('--node-modules', type=Path, default=ROOT / 'link/sdks/typescript/react/node_modules')
args = parser.parse_args()

def run(argv, cwd):
    subprocess.run([str(value) for value in argv], cwd=cwd, check=True)

with tempfile.TemporaryDirectory(prefix='kalam-sdk-contract-') as directory:
    project = Path(directory).resolve()
    shutil.copyfile(FIXTURES / 'schema.sql', project / 'schema.sql')
    (project / 'kalam.toml').write_text('''[project]
name = "sdk_contract_fixture"
[schema]
mode = "sql"
path = "schema.sql"
languages = ["typescript", "dart"]
[schema.targets.typescript]
output = "ts/kalam.ts"
[schema.targets.dart]
output = "lib/kalam.dart"
''')
    run([args.kalam, 'schema', 'gen', '--project-dir', project, '--languages', 'typescript,dart', '--agent', '--no-spinner'], ROOT)
    (project / 'package.json').write_text('{"type":"module"}')
    (project / 'node_modules').symlink_to(args.node_modules.resolve(), target_is_directory=True)
    shutil.copyfile(FIXTURES / 'shape-check.ts', project / 'ts/shape-check.ts')
    run([args.node_modules / '.bin/tsc', '--ignoreConfig', '--noEmit', '--strict', '--skipLibCheck', '--target', 'ES2022', '--module', 'NodeNext', '--moduleResolution', 'NodeNext', project / 'ts/shape-check.ts'], project)
    (project / 'pubspec.yaml').write_text(f"""name: sdk_contract_fixture
environment:
  sdk: ">=3.10.0 <4.0.0"
dependencies:
  kalam_sync:
    path: {ROOT / 'link/sdks/dart/sync'}
dependency_overrides:
  kalam_link:
    path: {ROOT / 'link/sdks/dart/link'}
dev_dependencies:
  flutter_test:
    sdk: flutter
""")
    run([args.flutter, 'pub', 'get', '--offline'], project)
    (project / 'test').mkdir()
    shutil.copyfile(FIXTURES / 'generated_test.dart', project / 'test/generated_test.dart')
    run([args.flutter, 'test', '--no-pub', 'test/generated_test.dart'], project)
    # Exercise the analyzer's negative type boundary without suppressing diagnostics.
    invalid = project / 'lib/invalid.dart'
    invalid.write_text("import 'kalam.dart';\nvoid wrongType() { InventoryColumns.sku.eq(42); }\n")
    flutter = Path(shutil.which(args.flutter) or args.flutter).resolve()
    dart = flutter.parent / 'cache/dart-sdk/bin/dart'
    result = subprocess.run([str(dart), 'analyze', str(invalid)], cwd=project, text=True, capture_output=True)
    if result.returncode == 0 or 'argument_type_not_assignable' not in result.stdout:
        raise RuntimeError(f'Expected generated Dart columns to reject wrong types:\n{result.stdout}\n{result.stderr}')
print('Generated SDK contracts: TypeScript typechecks, Dart round trips and negative types passed.')
