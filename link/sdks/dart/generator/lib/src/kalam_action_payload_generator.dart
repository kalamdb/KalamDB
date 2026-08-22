import 'package:analyzer/dart/element/element.dart';
import 'package:analyzer/dart/element/type.dart';
import 'package:build/build.dart';
import 'package:kalam_sync/src/annotations/kalam_action_payload.dart';
import 'package:source_gen/source_gen.dart';

final class KalamActionPayloadGenerator
    extends GeneratorForAnnotation<KalamActionPayload> {
  const KalamActionPayloadGenerator();

  @override
  String generateForAnnotatedElement(
    Element element,
    ConstantReader annotation,
    BuildStep buildStep,
  ) {
    if (element is! ClassElement) {
      throw InvalidGenerationSourceError(
        '@KalamActionPayload can only annotate a class.',
        element: element,
      );
    }
    final fields = element.fields.where((field) => !field.isStatic).toList();
    final constructor = element.unnamedConstructor;
    if (constructor == null ||
        constructor.formalParameters.any((parameter) => !parameter.isNamed)) {
      throw InvalidGenerationSourceError(
        '${element.name} needs an unnamed constructor with named parameters.',
        element: element,
      );
    }
    for (final field in fields) {
      _decode(field.type, 'json[\'${field.name}\']', element);
    }

    final name = element.name!;
    final fromFields = fields
        .map(
          (field) =>
              '${field.name}: ${_decode(field.type, "json['${field.name}']", element)},',
        )
        .join('\n');
    final toFields = fields
        .map(
          (field) =>
              "'${field.name}': ${_encode(field.type, 'value.${field.name}')},",
        )
        .join('\n');
    return '''
$name _\$${name}FromJson(Map<String, Object?> json) => $name(
$fromFields
);

Map<String, Object?> _\$${name}ToJson($name value) => {
$toFields
};
'''
        .trim();
  }

  String _decode(DartType type, String source, Element element) {
    final display = type.getDisplayString();
    final nullable = display.endsWith('?');
    final base = nullable ? display.substring(0, display.length - 1) : display;
    late String expression;
    if (base == 'String' || base == 'bool') {
      expression = '$source as $base';
    } else if (base == 'int') {
      expression = '($source as num).toInt()';
    } else if (base == 'double') {
      expression = '($source as num).toDouble()';
    } else if (base == 'DateTime') {
      expression = 'DateTime.parse($source as String)';
    } else if (base == 'List<String>') {
      expression = '($source as List).cast<String>()';
    } else if (base == 'Map<String, Object?>') {
      expression = 'Map<String, Object?>.from($source as Map)';
    } else {
      throw InvalidGenerationSourceError(
        'Unsupported durable payload field type "$display".',
        element: element,
      );
    }
    return nullable ? '$source == null ? null : $expression' : expression;
  }

  String _encode(DartType type, String source) {
    final display = type.getDisplayString();
    if (display == 'DateTime') return '$source.toUtc().toIso8601String()';
    if (display == 'DateTime?') return '$source?.toUtc().toIso8601String()';
    return source;
  }
}
