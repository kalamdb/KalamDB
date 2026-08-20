import 'dart:async';

import 'package:flutter/widgets.dart';

import '../kalam.dart';

/// Provides one [Kalam] session and coordinates it with Flutter lifecycle.
final class KalamScope extends StatefulWidget {
  const KalamScope({
    required this.kalam,
    required this.child,
    this.manageLifecycle = true,
    super.key,
  });

  final Kalam kalam;
  final Widget child;
  final bool manageLifecycle;

  static Kalam of(BuildContext context) {
    final scope = context.dependOnInheritedWidgetOfExactType<_KalamInherited>();
    return _requireScope(scope);
  }

  /// Reads the current session without rebuilding when the scope changes.
  ///
  /// This form is safe for one-time setup in `State.initState`.
  static Kalam read(BuildContext context) {
    final scope = context.getInheritedWidgetOfExactType<_KalamInherited>();
    return _requireScope(scope);
  }

  static Kalam _requireScope(_KalamInherited? scope) {
    if (scope == null) {
      throw FlutterError('No KalamScope was found above this context.');
    }
    return scope.kalam;
  }

  @override
  State<KalamScope> createState() => _KalamScopeState();
}

final class _KalamScopeState extends State<KalamScope>
    with WidgetsBindingObserver {
  @override
  void initState() {
    super.initState();
    if (widget.manageLifecycle) WidgetsBinding.instance.addObserver(this);
  }

  @override
  void didUpdateWidget(KalamScope oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.manageLifecycle == widget.manageLifecycle) return;
    if (widget.manageLifecycle) {
      WidgetsBinding.instance.addObserver(this);
    } else {
      WidgetsBinding.instance.removeObserver(this);
    }
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    switch (state) {
      case AppLifecycleState.resumed:
        unawaited(widget.kalam.resume());
      case AppLifecycleState.hidden || AppLifecycleState.paused:
        unawaited(widget.kalam.pause());
      case AppLifecycleState.inactive || AppLifecycleState.detached:
        break;
    }
  }

  @override
  void dispose() {
    if (widget.manageLifecycle) WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return _KalamInherited(kalam: widget.kalam, child: widget.child);
  }
}

final class _KalamInherited extends InheritedWidget {
  const _KalamInherited({required this.kalam, required super.child});

  final Kalam kalam;

  @override
  bool updateShouldNotify(_KalamInherited oldWidget) =>
      !identical(kalam, oldWidget.kalam);
}
