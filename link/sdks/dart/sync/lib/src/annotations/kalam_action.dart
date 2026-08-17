/// Marks one module method as a durable offline action.
final class KalamAction {
  const KalamAction({required this.name, this.version = 1});

  final String name;
  final int version;
}
