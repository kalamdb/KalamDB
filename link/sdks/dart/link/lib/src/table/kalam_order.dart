part of 'kalam_remote_table.dart';

final class KalamOrder {
  const KalamOrder(this.keys);

  final List<KalamOrderKey> keys;

  KalamOrder then(KalamOrder next) => KalamOrder([...keys, ...next.keys]);
}
