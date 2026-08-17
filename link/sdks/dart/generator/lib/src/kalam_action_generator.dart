import 'package:analyzer/dart/element/element.dart';
import 'package:build/build.dart';
import 'package:kalam_sync/src/annotations/kalam_action.dart';
import 'package:kalam_sync/src/annotations/kalam_action_module.dart';
import 'package:source_gen/source_gen.dart';

final class KalamActionGenerator
    extends GeneratorForAnnotation<KalamActionModule> {
  const KalamActionGenerator();

  static final _actionChecker = TypeChecker.typeNamed(
    KalamAction,
    inPackage: 'kalam_sync',
  );

  @override
  String generateForAnnotatedElement(
    Element element,
    ConstantReader annotation,
    BuildStep buildStep,
  ) {
    if (element is! ClassElement) {
      throw InvalidGenerationSourceError(
        '@KalamActionModule can only annotate a class.',
        element: element,
      );
    }
    final namespace = annotation.read('namespace').stringValue;
    final methods = <_ActionMethod>[];
    final keys = <String>{};
    for (final method in element.methods) {
      final value = _actionChecker.firstAnnotationOf(method);
      if (value == null) continue;
      final action = ConstantReader(value);
      final name = action.read('name').stringValue;
      final version = action.read('version').intValue;
      final key = '$namespace.$name';
      if (!keys.add(key)) {
        throw InvalidGenerationSourceError(
          'Duplicate action key "$key".',
          element: method,
        );
      }
      if (method.formalParameters.length != 2) {
        throw InvalidGenerationSourceError(
          '${method.name} must accept KalamActionContext and one payload.',
          element: method,
        );
      }
      final payloadType = method.formalParameters[1].type.getDisplayString();
      methods.add(_ActionMethod(method.name!, name, key, version, payloadType));
    }

    final moduleName = element.name!;
    final definitionsName =
        '${moduleName[0].toLowerCase()}${moduleName.substring(1)}Definitions';
    final definitions = methods
        .map(
          (method) =>
              '''
KalamActionDefinition<${method.payloadType}>(
  key: '${method.key}',
  version: ${method.version},
  codec: KalamActionCodec(
    encode: _\$${method.payloadType}ToJson,
    decode: _\$${method.payloadType}FromJson,
  ),
  execute: module.${method.methodName},
),''',
        )
        .join('\n');
    final queueMethods = methods
        .map(
          (method) =>
              '''
Future<KalamActionRecord> ${method.actionName}(
  ${method.payloadType} payload, {
  String? actionId,
  String? orderingKey,
  KalamOptimisticMutation? optimistic,
}) => _runner.enqueue(
  actionKey: '${method.key}',
  actionId: actionId ?? Kalam.id(),
  payload: payload,
  orderingKey: orderingKey,
  optimistic: optimistic,
);''',
        )
        .join('\n\n');

    return '''
List<KalamActionDefinition<dynamic>> $definitionsName($moduleName module) => [
$definitions
];

final class ${moduleName}Queue {
  const ${moduleName}Queue(this._runner);

  final KalamActionRunner _runner;

$queueMethods
}
'''
        .trim();
  }
}

final class _ActionMethod {
  const _ActionMethod(
    this.methodName,
    this.actionName,
    this.key,
    this.version,
    this.payloadType,
  );

  final String methodName;
  final String actionName;
  final String key;
  final int version;
  final String payloadType;
}
