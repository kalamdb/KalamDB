import 'kalam_action_definition.dart';

/// Immutable lookup of the app's registered action definitions.
final class KalamActionRegistry {
  KalamActionRegistry(Iterable<KalamActionDefinition<Object?>> actions)
    : _actions = _index(actions);

  factory KalamActionRegistry.of(
    Iterable<KalamActionDefinition<dynamic>> actions,
  ) {
    return KalamActionRegistry(actions.cast<KalamActionDefinition<Object?>>());
  }

  final Map<String, KalamActionDefinition<Object?>> _actions;

  KalamActionDefinition<Object?> operator [](String key) {
    final action = _actions[key];
    if (action == null) {
      throw StateError('No Kalam action is registered for "$key".');
    }
    return action;
  }

  static Map<String, KalamActionDefinition<Object?>> _index(
    Iterable<KalamActionDefinition<Object?>> actions,
  ) {
    final result = <String, KalamActionDefinition<Object?>>{};
    for (final action in actions) {
      if (result.containsKey(action.key)) {
        throw ArgumentError.value(
          action.key,
          'actions',
          'Duplicate action key',
        );
      }
      result[action.key] = action;
    }
    return Map.unmodifiable(result);
  }
}
