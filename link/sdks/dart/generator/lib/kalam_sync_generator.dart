import 'package:build/build.dart';
import 'package:source_gen/source_gen.dart';

import 'src/kalam_action_generator.dart';
import 'src/kalam_action_payload_generator.dart';

Builder kalamActionBuilder(BuilderOptions options) {
  return SharedPartBuilder(const [
    KalamActionPayloadGenerator(),
    KalamActionGenerator(),
  ], 'kalam_actions');
}
