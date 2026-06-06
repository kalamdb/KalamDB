# Contract: Generated Language Targets

## Purpose

This contract defines how schema-driven language generation behaves for the initial supported targets.

## Supported Targets in the First Release

- `typescript`
- `dart`

## Shared Rules

1. Generation is driven from the resolved schema source, not from handwritten target files.
2. Generated files are project artifacts and must not be edited directly.
3. A single project may enable one or more language targets.
4. Adding a future target must extend the contract without changing the meaning of existing targets.

## TypeScript Target

**Expected output**:
- A generated TypeScript file in the configured project path, for example `src/generated/kalam.ts`.

**Behavioral guarantees**:
- Output names are derived from the resolved schema model.
- Output is suitable for application developers to import directly.
- Generation remains separate from internal SDK-package build outputs.

## Dart Target

**Expected output**:
- A generated Dart file in the configured project path, for example `lib/generated/kalam.dart`.

**Behavioral guarantees**:
- Output names are derived from the resolved schema model.
- Output is suitable for application developers to import directly.
- Generation remains separate from existing Dart bridge-generated SDK files.

## Future Targets

Future language targets must:
- be declared under the same target-registration model
- provide their own output path
- follow the same generated-artifact and non-manual-edit rules
