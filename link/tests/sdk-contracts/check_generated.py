#!/usr/bin/env python3
"""Compile real generated TS/Dart/Python output against installed local SDK dependencies."""
import argparse
import json
import os
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
parser.add_argument('--python', action='store_true',
                    help='also typecheck generated Python; needs kalamdb installed (maturin develop) and mypy')
parser.add_argument('--python-bin', default='python3')
parser.add_argument('--skip-typescript', action='store_true',
                    help='skip the TypeScript target; it needs node_modules built')
parser.add_argument('--skip-dart', action='store_true',
                    help='skip the Dart target; it needs a Flutter toolchain')
args = parser.parse_args()
# Made absolute against the invoking directory: the Python steps run with `cwd`
# set to the generated output, where a relative path like .venv/bin/python would
# otherwise be looked up. A bare name such as python3 stays on PATH.
#
# absolute(), not resolve(): a venv's bin/python is a symlink to the system
# interpreter, and following it would run the checks outside the venv, without
# the kalamdb package installed.
if os.sep in args.python_bin or (os.altsep and os.altsep in args.python_bin):
    args.python_bin = str(Path(args.python_bin).absolute())

LANGUAGES = ([] if args.skip_typescript else ['typescript']) \
    + ([] if args.skip_dart else ['dart']) \
    + (['python'] if args.python else [])
if not LANGUAGES:
    raise SystemExit('nothing to check: every target was skipped')

def run(argv, cwd):
    subprocess.run([str(value) for value in argv], cwd=cwd, check=True)

with tempfile.TemporaryDirectory(prefix='kalam-sdk-contract-') as directory:
    project = Path(directory).resolve()
    shutil.copyfile(FIXTURES / 'schema.sql', project / 'schema.sql')
    (project / 'kalam.toml').write_text(f'''[project]
name = "sdk_contract_fixture"
[schema]
mode = "sql"
path = "schema.sql"
languages = {json.dumps(LANGUAGES)}
[schema.targets.typescript]
output = "ts/kalam.ts"
[schema.targets.dart]
output = "lib/kalam.dart"
[schema.targets.python]
output = "py/kalam.py"
''')
    run([args.kalam, 'schema', 'gen', '--project-dir', project, '--languages', ','.join(LANGUAGES), '--agent', '--no-spinner'], ROOT)
    if 'typescript' in LANGUAGES:
        (project / 'package.json').write_text('{"type":"module"}')
        (project / 'node_modules').symlink_to(args.node_modules.resolve(), target_is_directory=True)
        shutil.copyfile(FIXTURES / 'shape-check.ts', project / 'ts/shape-check.ts')
        run([args.node_modules / '.bin/tsc', '--ignoreConfig', '--noEmit', '--strict', '--skipLibCheck', '--target', 'ES2022', '--module', 'NodeNext', '--moduleResolution', 'NodeNext', project / 'ts/shape-check.ts'], project)
    if 'dart' in LANGUAGES:
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
    if args.python:
        # Generated Python is checked the same way: strict types on the real SDK,
        # a real import as the syntax gate, then a negative boundary mypy must reject.
        generated = project / 'py'
        shutil.copyfile(FIXTURES / 'shape_check.py', generated / 'shape_check.py')
        # mypy runs from the output directory. `schema gen` writes an __init__.py
        # beside kalam.py, so from the project root the module is `py.kalam` and
        # `from kalam import ...` picks up the CLI's `kalam/` working directory
        # instead of the generated bindings.
        run([args.python_bin, '-m', 'mypy', '--strict', '--no-error-summary',
             '--explicit-package-bases', 'kalam.py', 'shape_check.py'], generated)
        run([args.python_bin, '-c', 'import kalam; assert kalam.KALAM_TABLES'], generated)
        # mypy only typechecks the fixture; its assertions about table metadata,
        # enum values and the SQL builders need a real run.
        run([args.python_bin, 'shape_check.py'], generated)
        # Two negative boundaries: the row dataclass, and the write inputs.
        # The second is the one a plain Mapping signature used to wave through.
        for name, source, expected in [
            ('invalid_row.py',
             'from kalam import Inventory\n'
             'wrong = Inventory(id=1, sku="s", tags=None, dates=None,\n'
             '                  moods=None, addresses=None, attachment=None)\n',
             'arg-type'),
            ('invalid_insert.py',
             'from kalam import inventory\n'
             'sql, params = inventory.insert_sql({"id": 1, "sku": "s"})\n',
             'typeddict-item'),
            ('invalid_insert_key.py',
             'from kalam import profiles\n'
             'sql, params = profiles.insert_sql({"email": "a@b.c", "nope": 1})\n',
             'typeddict-unknown-key'),
            ('invalid_update.py',
             'from kalam import rooms\n'
             'sql, params = rooms.update_sql(1, {"name": 42})\n',
             'typeddict-item'),
        ]:
            (generated / name).write_text(source)
            result = subprocess.run([args.python_bin, '-m', 'mypy', '--strict',
                                     '--explicit-package-bases', name],
                                    cwd=generated, text=True, capture_output=True)
            if result.returncode == 0 or expected not in result.stdout:
                raise RuntimeError(
                    f'Expected generated Python to reject {name} with {expected}:\n'
                    f'{result.stdout}\n{result.stderr}')

checked = []
if 'typescript' in LANGUAGES:
    checked.append('TypeScript typechecks')
if 'dart' in LANGUAGES:
    checked.append('Dart round trips and rejects wrong types')
if 'python' in LANGUAGES:
    checked.append('Python typechecks, imports and rejects wrong types')
print('Generated SDK contracts: ' + '; '.join(checked) + '.')
