// GENERATED CODE - DO NOT MODIFY BY HAND

part of 'kalam_sync_database.dart';

// ignore_for_file: type=lint
class $KalamActionsTable extends KalamActions
    with TableInfo<$KalamActionsTable, StoredAction> {
  @override
  final GeneratedDatabase attachedDatabase;
  final String? _alias;
  $KalamActionsTable(this.attachedDatabase, [this._alias]);
  static const VerificationMeta _idMeta = const VerificationMeta('id');
  @override
  late final GeneratedColumn<String> id = GeneratedColumn<String>(
    'id',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _accountKeyMeta = const VerificationMeta(
    'accountKey',
  );
  @override
  late final GeneratedColumn<String> accountKey = GeneratedColumn<String>(
    'account_key',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _actionKeyMeta = const VerificationMeta(
    'actionKey',
  );
  @override
  late final GeneratedColumn<String> actionKey = GeneratedColumn<String>(
    'action_key',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _versionMeta = const VerificationMeta(
    'version',
  );
  @override
  late final GeneratedColumn<int> version = GeneratedColumn<int>(
    'version',
    aliasedName,
    false,
    type: DriftSqlType.int,
    requiredDuringInsert: false,
    defaultValue: const Constant(1),
  );
  static const VerificationMeta _kindMeta = const VerificationMeta('kind');
  @override
  late final GeneratedColumn<String> kind = GeneratedColumn<String>(
    'kind',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: false,
    defaultValue: const Constant('custom'),
  );
  static const VerificationMeta _payloadJsonMeta = const VerificationMeta(
    'payloadJson',
  );
  @override
  late final GeneratedColumn<String> payloadJson = GeneratedColumn<String>(
    'payload_json',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _statusMeta = const VerificationMeta('status');
  @override
  late final GeneratedColumn<String> status = GeneratedColumn<String>(
    'status',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _orderingKeyMeta = const VerificationMeta(
    'orderingKey',
  );
  @override
  late final GeneratedColumn<String> orderingKey = GeneratedColumn<String>(
    'ordering_key',
    aliasedName,
    true,
    type: DriftSqlType.string,
    requiredDuringInsert: false,
  );
  static const VerificationMeta _rowTableIdMeta = const VerificationMeta(
    'rowTableId',
  );
  @override
  late final GeneratedColumn<String> rowTableId = GeneratedColumn<String>(
    'row_table_id',
    aliasedName,
    true,
    type: DriftSqlType.string,
    requiredDuringInsert: false,
  );
  static const VerificationMeta _rowKeyMeta = const VerificationMeta('rowKey');
  @override
  late final GeneratedColumn<String> rowKey = GeneratedColumn<String>(
    'row_key',
    aliasedName,
    true,
    type: DriftSqlType.string,
    requiredDuringInsert: false,
  );
  static const VerificationMeta _queuePositionMeta = const VerificationMeta(
    'queuePosition',
  );
  @override
  late final GeneratedColumn<int> queuePosition = GeneratedColumn<int>(
    'queue_position',
    aliasedName,
    false,
    type: DriftSqlType.int,
    requiredDuringInsert: false,
    defaultValue: const Constant(0),
  );
  static const VerificationMeta _attemptCountMeta = const VerificationMeta(
    'attemptCount',
  );
  @override
  late final GeneratedColumn<int> attemptCount = GeneratedColumn<int>(
    'attempt_count',
    aliasedName,
    false,
    type: DriftSqlType.int,
    requiredDuringInsert: false,
    defaultValue: const Constant(0),
  );
  static const VerificationMeta _nextAttemptAtMeta = const VerificationMeta(
    'nextAttemptAt',
  );
  @override
  late final GeneratedColumn<DateTime> nextAttemptAt =
      GeneratedColumn<DateTime>(
        'next_attempt_at',
        aliasedName,
        true,
        type: DriftSqlType.dateTime,
        requiredDuringInsert: false,
      );
  static const VerificationMeta _lastErrorMeta = const VerificationMeta(
    'lastError',
  );
  @override
  late final GeneratedColumn<String> lastError = GeneratedColumn<String>(
    'last_error',
    aliasedName,
    true,
    type: DriftSqlType.string,
    requiredDuringInsert: false,
  );
  static const VerificationMeta _createdAtMeta = const VerificationMeta(
    'createdAt',
  );
  @override
  late final GeneratedColumn<DateTime> createdAt = GeneratedColumn<DateTime>(
    'created_at',
    aliasedName,
    false,
    type: DriftSqlType.dateTime,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _updatedAtMeta = const VerificationMeta(
    'updatedAt',
  );
  @override
  late final GeneratedColumn<DateTime> updatedAt = GeneratedColumn<DateTime>(
    'updated_at',
    aliasedName,
    false,
    type: DriftSqlType.dateTime,
    requiredDuringInsert: true,
  );
  @override
  List<GeneratedColumn> get $columns => [
    id,
    accountKey,
    actionKey,
    version,
    kind,
    payloadJson,
    status,
    orderingKey,
    rowTableId,
    rowKey,
    queuePosition,
    attemptCount,
    nextAttemptAt,
    lastError,
    createdAt,
    updatedAt,
  ];
  @override
  String get aliasedName => _alias ?? actualTableName;
  @override
  String get actualTableName => $name;
  static const String $name = 'kalam_actions';
  @override
  VerificationContext validateIntegrity(
    Insertable<StoredAction> instance, {
    bool isInserting = false,
  }) {
    final context = VerificationContext();
    final data = instance.toColumns(true);
    if (data.containsKey('id')) {
      context.handle(_idMeta, id.isAcceptableOrUnknown(data['id']!, _idMeta));
    } else if (isInserting) {
      context.missing(_idMeta);
    }
    if (data.containsKey('account_key')) {
      context.handle(
        _accountKeyMeta,
        accountKey.isAcceptableOrUnknown(data['account_key']!, _accountKeyMeta),
      );
    } else if (isInserting) {
      context.missing(_accountKeyMeta);
    }
    if (data.containsKey('action_key')) {
      context.handle(
        _actionKeyMeta,
        actionKey.isAcceptableOrUnknown(data['action_key']!, _actionKeyMeta),
      );
    } else if (isInserting) {
      context.missing(_actionKeyMeta);
    }
    if (data.containsKey('version')) {
      context.handle(
        _versionMeta,
        version.isAcceptableOrUnknown(data['version']!, _versionMeta),
      );
    }
    if (data.containsKey('kind')) {
      context.handle(
        _kindMeta,
        kind.isAcceptableOrUnknown(data['kind']!, _kindMeta),
      );
    }
    if (data.containsKey('payload_json')) {
      context.handle(
        _payloadJsonMeta,
        payloadJson.isAcceptableOrUnknown(
          data['payload_json']!,
          _payloadJsonMeta,
        ),
      );
    } else if (isInserting) {
      context.missing(_payloadJsonMeta);
    }
    if (data.containsKey('status')) {
      context.handle(
        _statusMeta,
        status.isAcceptableOrUnknown(data['status']!, _statusMeta),
      );
    } else if (isInserting) {
      context.missing(_statusMeta);
    }
    if (data.containsKey('ordering_key')) {
      context.handle(
        _orderingKeyMeta,
        orderingKey.isAcceptableOrUnknown(
          data['ordering_key']!,
          _orderingKeyMeta,
        ),
      );
    }
    if (data.containsKey('row_table_id')) {
      context.handle(
        _rowTableIdMeta,
        rowTableId.isAcceptableOrUnknown(
          data['row_table_id']!,
          _rowTableIdMeta,
        ),
      );
    }
    if (data.containsKey('row_key')) {
      context.handle(
        _rowKeyMeta,
        rowKey.isAcceptableOrUnknown(data['row_key']!, _rowKeyMeta),
      );
    }
    if (data.containsKey('queue_position')) {
      context.handle(
        _queuePositionMeta,
        queuePosition.isAcceptableOrUnknown(
          data['queue_position']!,
          _queuePositionMeta,
        ),
      );
    }
    if (data.containsKey('attempt_count')) {
      context.handle(
        _attemptCountMeta,
        attemptCount.isAcceptableOrUnknown(
          data['attempt_count']!,
          _attemptCountMeta,
        ),
      );
    }
    if (data.containsKey('next_attempt_at')) {
      context.handle(
        _nextAttemptAtMeta,
        nextAttemptAt.isAcceptableOrUnknown(
          data['next_attempt_at']!,
          _nextAttemptAtMeta,
        ),
      );
    }
    if (data.containsKey('last_error')) {
      context.handle(
        _lastErrorMeta,
        lastError.isAcceptableOrUnknown(data['last_error']!, _lastErrorMeta),
      );
    }
    if (data.containsKey('created_at')) {
      context.handle(
        _createdAtMeta,
        createdAt.isAcceptableOrUnknown(data['created_at']!, _createdAtMeta),
      );
    } else if (isInserting) {
      context.missing(_createdAtMeta);
    }
    if (data.containsKey('updated_at')) {
      context.handle(
        _updatedAtMeta,
        updatedAt.isAcceptableOrUnknown(data['updated_at']!, _updatedAtMeta),
      );
    } else if (isInserting) {
      context.missing(_updatedAtMeta);
    }
    return context;
  }

  @override
  Set<GeneratedColumn> get $primaryKey => {id};
  @override
  StoredAction map(Map<String, dynamic> data, {String? tablePrefix}) {
    final effectivePrefix = tablePrefix != null ? '$tablePrefix.' : '';
    return StoredAction(
      id: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}id'],
      )!,
      accountKey: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}account_key'],
      )!,
      actionKey: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}action_key'],
      )!,
      version: attachedDatabase.typeMapping.read(
        DriftSqlType.int,
        data['${effectivePrefix}version'],
      )!,
      kind: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}kind'],
      )!,
      payloadJson: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}payload_json'],
      )!,
      status: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}status'],
      )!,
      orderingKey: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}ordering_key'],
      ),
      rowTableId: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}row_table_id'],
      ),
      rowKey: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}row_key'],
      ),
      queuePosition: attachedDatabase.typeMapping.read(
        DriftSqlType.int,
        data['${effectivePrefix}queue_position'],
      )!,
      attemptCount: attachedDatabase.typeMapping.read(
        DriftSqlType.int,
        data['${effectivePrefix}attempt_count'],
      )!,
      nextAttemptAt: attachedDatabase.typeMapping.read(
        DriftSqlType.dateTime,
        data['${effectivePrefix}next_attempt_at'],
      ),
      lastError: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}last_error'],
      ),
      createdAt: attachedDatabase.typeMapping.read(
        DriftSqlType.dateTime,
        data['${effectivePrefix}created_at'],
      )!,
      updatedAt: attachedDatabase.typeMapping.read(
        DriftSqlType.dateTime,
        data['${effectivePrefix}updated_at'],
      )!,
    );
  }

  @override
  $KalamActionsTable createAlias(String alias) {
    return $KalamActionsTable(attachedDatabase, alias);
  }
}

class StoredAction extends DataClass implements Insertable<StoredAction> {
  final String id;
  final String accountKey;
  final String actionKey;
  final int version;
  final String kind;
  final String payloadJson;
  final String status;
  final String? orderingKey;
  final String? rowTableId;
  final String? rowKey;
  final int queuePosition;
  final int attemptCount;
  final DateTime? nextAttemptAt;
  final String? lastError;
  final DateTime createdAt;
  final DateTime updatedAt;
  const StoredAction({
    required this.id,
    required this.accountKey,
    required this.actionKey,
    required this.version,
    required this.kind,
    required this.payloadJson,
    required this.status,
    this.orderingKey,
    this.rowTableId,
    this.rowKey,
    required this.queuePosition,
    required this.attemptCount,
    this.nextAttemptAt,
    this.lastError,
    required this.createdAt,
    required this.updatedAt,
  });
  @override
  Map<String, Expression> toColumns(bool nullToAbsent) {
    final map = <String, Expression>{};
    map['id'] = Variable<String>(id);
    map['account_key'] = Variable<String>(accountKey);
    map['action_key'] = Variable<String>(actionKey);
    map['version'] = Variable<int>(version);
    map['kind'] = Variable<String>(kind);
    map['payload_json'] = Variable<String>(payloadJson);
    map['status'] = Variable<String>(status);
    if (!nullToAbsent || orderingKey != null) {
      map['ordering_key'] = Variable<String>(orderingKey);
    }
    if (!nullToAbsent || rowTableId != null) {
      map['row_table_id'] = Variable<String>(rowTableId);
    }
    if (!nullToAbsent || rowKey != null) {
      map['row_key'] = Variable<String>(rowKey);
    }
    map['queue_position'] = Variable<int>(queuePosition);
    map['attempt_count'] = Variable<int>(attemptCount);
    if (!nullToAbsent || nextAttemptAt != null) {
      map['next_attempt_at'] = Variable<DateTime>(nextAttemptAt);
    }
    if (!nullToAbsent || lastError != null) {
      map['last_error'] = Variable<String>(lastError);
    }
    map['created_at'] = Variable<DateTime>(createdAt);
    map['updated_at'] = Variable<DateTime>(updatedAt);
    return map;
  }

  KalamActionsCompanion toCompanion(bool nullToAbsent) {
    return KalamActionsCompanion(
      id: Value(id),
      accountKey: Value(accountKey),
      actionKey: Value(actionKey),
      version: Value(version),
      kind: Value(kind),
      payloadJson: Value(payloadJson),
      status: Value(status),
      orderingKey: orderingKey == null && nullToAbsent
          ? const Value.absent()
          : Value(orderingKey),
      rowTableId: rowTableId == null && nullToAbsent
          ? const Value.absent()
          : Value(rowTableId),
      rowKey: rowKey == null && nullToAbsent
          ? const Value.absent()
          : Value(rowKey),
      queuePosition: Value(queuePosition),
      attemptCount: Value(attemptCount),
      nextAttemptAt: nextAttemptAt == null && nullToAbsent
          ? const Value.absent()
          : Value(nextAttemptAt),
      lastError: lastError == null && nullToAbsent
          ? const Value.absent()
          : Value(lastError),
      createdAt: Value(createdAt),
      updatedAt: Value(updatedAt),
    );
  }

  factory StoredAction.fromJson(
    Map<String, dynamic> json, {
    ValueSerializer? serializer,
  }) {
    serializer ??= driftRuntimeOptions.defaultSerializer;
    return StoredAction(
      id: serializer.fromJson<String>(json['id']),
      accountKey: serializer.fromJson<String>(json['accountKey']),
      actionKey: serializer.fromJson<String>(json['actionKey']),
      version: serializer.fromJson<int>(json['version']),
      kind: serializer.fromJson<String>(json['kind']),
      payloadJson: serializer.fromJson<String>(json['payloadJson']),
      status: serializer.fromJson<String>(json['status']),
      orderingKey: serializer.fromJson<String?>(json['orderingKey']),
      rowTableId: serializer.fromJson<String?>(json['rowTableId']),
      rowKey: serializer.fromJson<String?>(json['rowKey']),
      queuePosition: serializer.fromJson<int>(json['queuePosition']),
      attemptCount: serializer.fromJson<int>(json['attemptCount']),
      nextAttemptAt: serializer.fromJson<DateTime?>(json['nextAttemptAt']),
      lastError: serializer.fromJson<String?>(json['lastError']),
      createdAt: serializer.fromJson<DateTime>(json['createdAt']),
      updatedAt: serializer.fromJson<DateTime>(json['updatedAt']),
    );
  }
  @override
  Map<String, dynamic> toJson({ValueSerializer? serializer}) {
    serializer ??= driftRuntimeOptions.defaultSerializer;
    return <String, dynamic>{
      'id': serializer.toJson<String>(id),
      'accountKey': serializer.toJson<String>(accountKey),
      'actionKey': serializer.toJson<String>(actionKey),
      'version': serializer.toJson<int>(version),
      'kind': serializer.toJson<String>(kind),
      'payloadJson': serializer.toJson<String>(payloadJson),
      'status': serializer.toJson<String>(status),
      'orderingKey': serializer.toJson<String?>(orderingKey),
      'rowTableId': serializer.toJson<String?>(rowTableId),
      'rowKey': serializer.toJson<String?>(rowKey),
      'queuePosition': serializer.toJson<int>(queuePosition),
      'attemptCount': serializer.toJson<int>(attemptCount),
      'nextAttemptAt': serializer.toJson<DateTime?>(nextAttemptAt),
      'lastError': serializer.toJson<String?>(lastError),
      'createdAt': serializer.toJson<DateTime>(createdAt),
      'updatedAt': serializer.toJson<DateTime>(updatedAt),
    };
  }

  StoredAction copyWith({
    String? id,
    String? accountKey,
    String? actionKey,
    int? version,
    String? kind,
    String? payloadJson,
    String? status,
    Value<String?> orderingKey = const Value.absent(),
    Value<String?> rowTableId = const Value.absent(),
    Value<String?> rowKey = const Value.absent(),
    int? queuePosition,
    int? attemptCount,
    Value<DateTime?> nextAttemptAt = const Value.absent(),
    Value<String?> lastError = const Value.absent(),
    DateTime? createdAt,
    DateTime? updatedAt,
  }) => StoredAction(
    id: id ?? this.id,
    accountKey: accountKey ?? this.accountKey,
    actionKey: actionKey ?? this.actionKey,
    version: version ?? this.version,
    kind: kind ?? this.kind,
    payloadJson: payloadJson ?? this.payloadJson,
    status: status ?? this.status,
    orderingKey: orderingKey.present ? orderingKey.value : this.orderingKey,
    rowTableId: rowTableId.present ? rowTableId.value : this.rowTableId,
    rowKey: rowKey.present ? rowKey.value : this.rowKey,
    queuePosition: queuePosition ?? this.queuePosition,
    attemptCount: attemptCount ?? this.attemptCount,
    nextAttemptAt: nextAttemptAt.present
        ? nextAttemptAt.value
        : this.nextAttemptAt,
    lastError: lastError.present ? lastError.value : this.lastError,
    createdAt: createdAt ?? this.createdAt,
    updatedAt: updatedAt ?? this.updatedAt,
  );
  StoredAction copyWithCompanion(KalamActionsCompanion data) {
    return StoredAction(
      id: data.id.present ? data.id.value : this.id,
      accountKey: data.accountKey.present
          ? data.accountKey.value
          : this.accountKey,
      actionKey: data.actionKey.present ? data.actionKey.value : this.actionKey,
      version: data.version.present ? data.version.value : this.version,
      kind: data.kind.present ? data.kind.value : this.kind,
      payloadJson: data.payloadJson.present
          ? data.payloadJson.value
          : this.payloadJson,
      status: data.status.present ? data.status.value : this.status,
      orderingKey: data.orderingKey.present
          ? data.orderingKey.value
          : this.orderingKey,
      rowTableId: data.rowTableId.present
          ? data.rowTableId.value
          : this.rowTableId,
      rowKey: data.rowKey.present ? data.rowKey.value : this.rowKey,
      queuePosition: data.queuePosition.present
          ? data.queuePosition.value
          : this.queuePosition,
      attemptCount: data.attemptCount.present
          ? data.attemptCount.value
          : this.attemptCount,
      nextAttemptAt: data.nextAttemptAt.present
          ? data.nextAttemptAt.value
          : this.nextAttemptAt,
      lastError: data.lastError.present ? data.lastError.value : this.lastError,
      createdAt: data.createdAt.present ? data.createdAt.value : this.createdAt,
      updatedAt: data.updatedAt.present ? data.updatedAt.value : this.updatedAt,
    );
  }

  @override
  String toString() {
    return (StringBuffer('StoredAction(')
          ..write('id: $id, ')
          ..write('accountKey: $accountKey, ')
          ..write('actionKey: $actionKey, ')
          ..write('version: $version, ')
          ..write('kind: $kind, ')
          ..write('payloadJson: $payloadJson, ')
          ..write('status: $status, ')
          ..write('orderingKey: $orderingKey, ')
          ..write('rowTableId: $rowTableId, ')
          ..write('rowKey: $rowKey, ')
          ..write('queuePosition: $queuePosition, ')
          ..write('attemptCount: $attemptCount, ')
          ..write('nextAttemptAt: $nextAttemptAt, ')
          ..write('lastError: $lastError, ')
          ..write('createdAt: $createdAt, ')
          ..write('updatedAt: $updatedAt')
          ..write(')'))
        .toString();
  }

  @override
  int get hashCode => Object.hash(
    id,
    accountKey,
    actionKey,
    version,
    kind,
    payloadJson,
    status,
    orderingKey,
    rowTableId,
    rowKey,
    queuePosition,
    attemptCount,
    nextAttemptAt,
    lastError,
    createdAt,
    updatedAt,
  );
  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      (other is StoredAction &&
          other.id == this.id &&
          other.accountKey == this.accountKey &&
          other.actionKey == this.actionKey &&
          other.version == this.version &&
          other.kind == this.kind &&
          other.payloadJson == this.payloadJson &&
          other.status == this.status &&
          other.orderingKey == this.orderingKey &&
          other.rowTableId == this.rowTableId &&
          other.rowKey == this.rowKey &&
          other.queuePosition == this.queuePosition &&
          other.attemptCount == this.attemptCount &&
          other.nextAttemptAt == this.nextAttemptAt &&
          other.lastError == this.lastError &&
          other.createdAt == this.createdAt &&
          other.updatedAt == this.updatedAt);
}

class KalamActionsCompanion extends UpdateCompanion<StoredAction> {
  final Value<String> id;
  final Value<String> accountKey;
  final Value<String> actionKey;
  final Value<int> version;
  final Value<String> kind;
  final Value<String> payloadJson;
  final Value<String> status;
  final Value<String?> orderingKey;
  final Value<String?> rowTableId;
  final Value<String?> rowKey;
  final Value<int> queuePosition;
  final Value<int> attemptCount;
  final Value<DateTime?> nextAttemptAt;
  final Value<String?> lastError;
  final Value<DateTime> createdAt;
  final Value<DateTime> updatedAt;
  final Value<int> rowid;
  const KalamActionsCompanion({
    this.id = const Value.absent(),
    this.accountKey = const Value.absent(),
    this.actionKey = const Value.absent(),
    this.version = const Value.absent(),
    this.kind = const Value.absent(),
    this.payloadJson = const Value.absent(),
    this.status = const Value.absent(),
    this.orderingKey = const Value.absent(),
    this.rowTableId = const Value.absent(),
    this.rowKey = const Value.absent(),
    this.queuePosition = const Value.absent(),
    this.attemptCount = const Value.absent(),
    this.nextAttemptAt = const Value.absent(),
    this.lastError = const Value.absent(),
    this.createdAt = const Value.absent(),
    this.updatedAt = const Value.absent(),
    this.rowid = const Value.absent(),
  });
  KalamActionsCompanion.insert({
    required String id,
    required String accountKey,
    required String actionKey,
    this.version = const Value.absent(),
    this.kind = const Value.absent(),
    required String payloadJson,
    required String status,
    this.orderingKey = const Value.absent(),
    this.rowTableId = const Value.absent(),
    this.rowKey = const Value.absent(),
    this.queuePosition = const Value.absent(),
    this.attemptCount = const Value.absent(),
    this.nextAttemptAt = const Value.absent(),
    this.lastError = const Value.absent(),
    required DateTime createdAt,
    required DateTime updatedAt,
    this.rowid = const Value.absent(),
  }) : id = Value(id),
       accountKey = Value(accountKey),
       actionKey = Value(actionKey),
       payloadJson = Value(payloadJson),
       status = Value(status),
       createdAt = Value(createdAt),
       updatedAt = Value(updatedAt);
  static Insertable<StoredAction> custom({
    Expression<String>? id,
    Expression<String>? accountKey,
    Expression<String>? actionKey,
    Expression<int>? version,
    Expression<String>? kind,
    Expression<String>? payloadJson,
    Expression<String>? status,
    Expression<String>? orderingKey,
    Expression<String>? rowTableId,
    Expression<String>? rowKey,
    Expression<int>? queuePosition,
    Expression<int>? attemptCount,
    Expression<DateTime>? nextAttemptAt,
    Expression<String>? lastError,
    Expression<DateTime>? createdAt,
    Expression<DateTime>? updatedAt,
    Expression<int>? rowid,
  }) {
    return RawValuesInsertable({
      if (id != null) 'id': id,
      if (accountKey != null) 'account_key': accountKey,
      if (actionKey != null) 'action_key': actionKey,
      if (version != null) 'version': version,
      if (kind != null) 'kind': kind,
      if (payloadJson != null) 'payload_json': payloadJson,
      if (status != null) 'status': status,
      if (orderingKey != null) 'ordering_key': orderingKey,
      if (rowTableId != null) 'row_table_id': rowTableId,
      if (rowKey != null) 'row_key': rowKey,
      if (queuePosition != null) 'queue_position': queuePosition,
      if (attemptCount != null) 'attempt_count': attemptCount,
      if (nextAttemptAt != null) 'next_attempt_at': nextAttemptAt,
      if (lastError != null) 'last_error': lastError,
      if (createdAt != null) 'created_at': createdAt,
      if (updatedAt != null) 'updated_at': updatedAt,
      if (rowid != null) 'rowid': rowid,
    });
  }

  KalamActionsCompanion copyWith({
    Value<String>? id,
    Value<String>? accountKey,
    Value<String>? actionKey,
    Value<int>? version,
    Value<String>? kind,
    Value<String>? payloadJson,
    Value<String>? status,
    Value<String?>? orderingKey,
    Value<String?>? rowTableId,
    Value<String?>? rowKey,
    Value<int>? queuePosition,
    Value<int>? attemptCount,
    Value<DateTime?>? nextAttemptAt,
    Value<String?>? lastError,
    Value<DateTime>? createdAt,
    Value<DateTime>? updatedAt,
    Value<int>? rowid,
  }) {
    return KalamActionsCompanion(
      id: id ?? this.id,
      accountKey: accountKey ?? this.accountKey,
      actionKey: actionKey ?? this.actionKey,
      version: version ?? this.version,
      kind: kind ?? this.kind,
      payloadJson: payloadJson ?? this.payloadJson,
      status: status ?? this.status,
      orderingKey: orderingKey ?? this.orderingKey,
      rowTableId: rowTableId ?? this.rowTableId,
      rowKey: rowKey ?? this.rowKey,
      queuePosition: queuePosition ?? this.queuePosition,
      attemptCount: attemptCount ?? this.attemptCount,
      nextAttemptAt: nextAttemptAt ?? this.nextAttemptAt,
      lastError: lastError ?? this.lastError,
      createdAt: createdAt ?? this.createdAt,
      updatedAt: updatedAt ?? this.updatedAt,
      rowid: rowid ?? this.rowid,
    );
  }

  @override
  Map<String, Expression> toColumns(bool nullToAbsent) {
    final map = <String, Expression>{};
    if (id.present) {
      map['id'] = Variable<String>(id.value);
    }
    if (accountKey.present) {
      map['account_key'] = Variable<String>(accountKey.value);
    }
    if (actionKey.present) {
      map['action_key'] = Variable<String>(actionKey.value);
    }
    if (version.present) {
      map['version'] = Variable<int>(version.value);
    }
    if (kind.present) {
      map['kind'] = Variable<String>(kind.value);
    }
    if (payloadJson.present) {
      map['payload_json'] = Variable<String>(payloadJson.value);
    }
    if (status.present) {
      map['status'] = Variable<String>(status.value);
    }
    if (orderingKey.present) {
      map['ordering_key'] = Variable<String>(orderingKey.value);
    }
    if (rowTableId.present) {
      map['row_table_id'] = Variable<String>(rowTableId.value);
    }
    if (rowKey.present) {
      map['row_key'] = Variable<String>(rowKey.value);
    }
    if (queuePosition.present) {
      map['queue_position'] = Variable<int>(queuePosition.value);
    }
    if (attemptCount.present) {
      map['attempt_count'] = Variable<int>(attemptCount.value);
    }
    if (nextAttemptAt.present) {
      map['next_attempt_at'] = Variable<DateTime>(nextAttemptAt.value);
    }
    if (lastError.present) {
      map['last_error'] = Variable<String>(lastError.value);
    }
    if (createdAt.present) {
      map['created_at'] = Variable<DateTime>(createdAt.value);
    }
    if (updatedAt.present) {
      map['updated_at'] = Variable<DateTime>(updatedAt.value);
    }
    if (rowid.present) {
      map['rowid'] = Variable<int>(rowid.value);
    }
    return map;
  }

  @override
  String toString() {
    return (StringBuffer('KalamActionsCompanion(')
          ..write('id: $id, ')
          ..write('accountKey: $accountKey, ')
          ..write('actionKey: $actionKey, ')
          ..write('version: $version, ')
          ..write('kind: $kind, ')
          ..write('payloadJson: $payloadJson, ')
          ..write('status: $status, ')
          ..write('orderingKey: $orderingKey, ')
          ..write('rowTableId: $rowTableId, ')
          ..write('rowKey: $rowKey, ')
          ..write('queuePosition: $queuePosition, ')
          ..write('attemptCount: $attemptCount, ')
          ..write('nextAttemptAt: $nextAttemptAt, ')
          ..write('lastError: $lastError, ')
          ..write('createdAt: $createdAt, ')
          ..write('updatedAt: $updatedAt, ')
          ..write('rowid: $rowid')
          ..write(')'))
        .toString();
  }
}

class $KalamCachedRowsTable extends KalamCachedRows
    with TableInfo<$KalamCachedRowsTable, StoredCachedRow> {
  @override
  final GeneratedDatabase attachedDatabase;
  final String? _alias;
  $KalamCachedRowsTable(this.attachedDatabase, [this._alias]);
  static const VerificationMeta _accountKeyMeta = const VerificationMeta(
    'accountKey',
  );
  @override
  late final GeneratedColumn<String> accountKey = GeneratedColumn<String>(
    'account_key',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _tableIdMeta = const VerificationMeta(
    'tableId',
  );
  @override
  late final GeneratedColumn<String> tableId = GeneratedColumn<String>(
    'table_id',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _rowKeyMeta = const VerificationMeta('rowKey');
  @override
  late final GeneratedColumn<String> rowKey = GeneratedColumn<String>(
    'row_key',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _valuesJsonMeta = const VerificationMeta(
    'valuesJson',
  );
  @override
  late final GeneratedColumn<String> valuesJson = GeneratedColumn<String>(
    'values_json',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _updatedAtMeta = const VerificationMeta(
    'updatedAt',
  );
  @override
  late final GeneratedColumn<DateTime> updatedAt = GeneratedColumn<DateTime>(
    'updated_at',
    aliasedName,
    false,
    type: DriftSqlType.dateTime,
    requiredDuringInsert: true,
  );
  @override
  List<GeneratedColumn> get $columns => [
    accountKey,
    tableId,
    rowKey,
    valuesJson,
    updatedAt,
  ];
  @override
  String get aliasedName => _alias ?? actualTableName;
  @override
  String get actualTableName => $name;
  static const String $name = 'kalam_cached_rows';
  @override
  VerificationContext validateIntegrity(
    Insertable<StoredCachedRow> instance, {
    bool isInserting = false,
  }) {
    final context = VerificationContext();
    final data = instance.toColumns(true);
    if (data.containsKey('account_key')) {
      context.handle(
        _accountKeyMeta,
        accountKey.isAcceptableOrUnknown(data['account_key']!, _accountKeyMeta),
      );
    } else if (isInserting) {
      context.missing(_accountKeyMeta);
    }
    if (data.containsKey('table_id')) {
      context.handle(
        _tableIdMeta,
        tableId.isAcceptableOrUnknown(data['table_id']!, _tableIdMeta),
      );
    } else if (isInserting) {
      context.missing(_tableIdMeta);
    }
    if (data.containsKey('row_key')) {
      context.handle(
        _rowKeyMeta,
        rowKey.isAcceptableOrUnknown(data['row_key']!, _rowKeyMeta),
      );
    } else if (isInserting) {
      context.missing(_rowKeyMeta);
    }
    if (data.containsKey('values_json')) {
      context.handle(
        _valuesJsonMeta,
        valuesJson.isAcceptableOrUnknown(data['values_json']!, _valuesJsonMeta),
      );
    } else if (isInserting) {
      context.missing(_valuesJsonMeta);
    }
    if (data.containsKey('updated_at')) {
      context.handle(
        _updatedAtMeta,
        updatedAt.isAcceptableOrUnknown(data['updated_at']!, _updatedAtMeta),
      );
    } else if (isInserting) {
      context.missing(_updatedAtMeta);
    }
    return context;
  }

  @override
  Set<GeneratedColumn> get $primaryKey => {accountKey, tableId, rowKey};
  @override
  StoredCachedRow map(Map<String, dynamic> data, {String? tablePrefix}) {
    final effectivePrefix = tablePrefix != null ? '$tablePrefix.' : '';
    return StoredCachedRow(
      accountKey: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}account_key'],
      )!,
      tableId: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}table_id'],
      )!,
      rowKey: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}row_key'],
      )!,
      valuesJson: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}values_json'],
      )!,
      updatedAt: attachedDatabase.typeMapping.read(
        DriftSqlType.dateTime,
        data['${effectivePrefix}updated_at'],
      )!,
    );
  }

  @override
  $KalamCachedRowsTable createAlias(String alias) {
    return $KalamCachedRowsTable(attachedDatabase, alias);
  }
}

class StoredCachedRow extends DataClass implements Insertable<StoredCachedRow> {
  final String accountKey;
  final String tableId;
  final String rowKey;
  final String valuesJson;
  final DateTime updatedAt;
  const StoredCachedRow({
    required this.accountKey,
    required this.tableId,
    required this.rowKey,
    required this.valuesJson,
    required this.updatedAt,
  });
  @override
  Map<String, Expression> toColumns(bool nullToAbsent) {
    final map = <String, Expression>{};
    map['account_key'] = Variable<String>(accountKey);
    map['table_id'] = Variable<String>(tableId);
    map['row_key'] = Variable<String>(rowKey);
    map['values_json'] = Variable<String>(valuesJson);
    map['updated_at'] = Variable<DateTime>(updatedAt);
    return map;
  }

  KalamCachedRowsCompanion toCompanion(bool nullToAbsent) {
    return KalamCachedRowsCompanion(
      accountKey: Value(accountKey),
      tableId: Value(tableId),
      rowKey: Value(rowKey),
      valuesJson: Value(valuesJson),
      updatedAt: Value(updatedAt),
    );
  }

  factory StoredCachedRow.fromJson(
    Map<String, dynamic> json, {
    ValueSerializer? serializer,
  }) {
    serializer ??= driftRuntimeOptions.defaultSerializer;
    return StoredCachedRow(
      accountKey: serializer.fromJson<String>(json['accountKey']),
      tableId: serializer.fromJson<String>(json['tableId']),
      rowKey: serializer.fromJson<String>(json['rowKey']),
      valuesJson: serializer.fromJson<String>(json['valuesJson']),
      updatedAt: serializer.fromJson<DateTime>(json['updatedAt']),
    );
  }
  @override
  Map<String, dynamic> toJson({ValueSerializer? serializer}) {
    serializer ??= driftRuntimeOptions.defaultSerializer;
    return <String, dynamic>{
      'accountKey': serializer.toJson<String>(accountKey),
      'tableId': serializer.toJson<String>(tableId),
      'rowKey': serializer.toJson<String>(rowKey),
      'valuesJson': serializer.toJson<String>(valuesJson),
      'updatedAt': serializer.toJson<DateTime>(updatedAt),
    };
  }

  StoredCachedRow copyWith({
    String? accountKey,
    String? tableId,
    String? rowKey,
    String? valuesJson,
    DateTime? updatedAt,
  }) => StoredCachedRow(
    accountKey: accountKey ?? this.accountKey,
    tableId: tableId ?? this.tableId,
    rowKey: rowKey ?? this.rowKey,
    valuesJson: valuesJson ?? this.valuesJson,
    updatedAt: updatedAt ?? this.updatedAt,
  );
  StoredCachedRow copyWithCompanion(KalamCachedRowsCompanion data) {
    return StoredCachedRow(
      accountKey: data.accountKey.present
          ? data.accountKey.value
          : this.accountKey,
      tableId: data.tableId.present ? data.tableId.value : this.tableId,
      rowKey: data.rowKey.present ? data.rowKey.value : this.rowKey,
      valuesJson: data.valuesJson.present
          ? data.valuesJson.value
          : this.valuesJson,
      updatedAt: data.updatedAt.present ? data.updatedAt.value : this.updatedAt,
    );
  }

  @override
  String toString() {
    return (StringBuffer('StoredCachedRow(')
          ..write('accountKey: $accountKey, ')
          ..write('tableId: $tableId, ')
          ..write('rowKey: $rowKey, ')
          ..write('valuesJson: $valuesJson, ')
          ..write('updatedAt: $updatedAt')
          ..write(')'))
        .toString();
  }

  @override
  int get hashCode =>
      Object.hash(accountKey, tableId, rowKey, valuesJson, updatedAt);
  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      (other is StoredCachedRow &&
          other.accountKey == this.accountKey &&
          other.tableId == this.tableId &&
          other.rowKey == this.rowKey &&
          other.valuesJson == this.valuesJson &&
          other.updatedAt == this.updatedAt);
}

class KalamCachedRowsCompanion extends UpdateCompanion<StoredCachedRow> {
  final Value<String> accountKey;
  final Value<String> tableId;
  final Value<String> rowKey;
  final Value<String> valuesJson;
  final Value<DateTime> updatedAt;
  final Value<int> rowid;
  const KalamCachedRowsCompanion({
    this.accountKey = const Value.absent(),
    this.tableId = const Value.absent(),
    this.rowKey = const Value.absent(),
    this.valuesJson = const Value.absent(),
    this.updatedAt = const Value.absent(),
    this.rowid = const Value.absent(),
  });
  KalamCachedRowsCompanion.insert({
    required String accountKey,
    required String tableId,
    required String rowKey,
    required String valuesJson,
    required DateTime updatedAt,
    this.rowid = const Value.absent(),
  }) : accountKey = Value(accountKey),
       tableId = Value(tableId),
       rowKey = Value(rowKey),
       valuesJson = Value(valuesJson),
       updatedAt = Value(updatedAt);
  static Insertable<StoredCachedRow> custom({
    Expression<String>? accountKey,
    Expression<String>? tableId,
    Expression<String>? rowKey,
    Expression<String>? valuesJson,
    Expression<DateTime>? updatedAt,
    Expression<int>? rowid,
  }) {
    return RawValuesInsertable({
      if (accountKey != null) 'account_key': accountKey,
      if (tableId != null) 'table_id': tableId,
      if (rowKey != null) 'row_key': rowKey,
      if (valuesJson != null) 'values_json': valuesJson,
      if (updatedAt != null) 'updated_at': updatedAt,
      if (rowid != null) 'rowid': rowid,
    });
  }

  KalamCachedRowsCompanion copyWith({
    Value<String>? accountKey,
    Value<String>? tableId,
    Value<String>? rowKey,
    Value<String>? valuesJson,
    Value<DateTime>? updatedAt,
    Value<int>? rowid,
  }) {
    return KalamCachedRowsCompanion(
      accountKey: accountKey ?? this.accountKey,
      tableId: tableId ?? this.tableId,
      rowKey: rowKey ?? this.rowKey,
      valuesJson: valuesJson ?? this.valuesJson,
      updatedAt: updatedAt ?? this.updatedAt,
      rowid: rowid ?? this.rowid,
    );
  }

  @override
  Map<String, Expression> toColumns(bool nullToAbsent) {
    final map = <String, Expression>{};
    if (accountKey.present) {
      map['account_key'] = Variable<String>(accountKey.value);
    }
    if (tableId.present) {
      map['table_id'] = Variable<String>(tableId.value);
    }
    if (rowKey.present) {
      map['row_key'] = Variable<String>(rowKey.value);
    }
    if (valuesJson.present) {
      map['values_json'] = Variable<String>(valuesJson.value);
    }
    if (updatedAt.present) {
      map['updated_at'] = Variable<DateTime>(updatedAt.value);
    }
    if (rowid.present) {
      map['rowid'] = Variable<int>(rowid.value);
    }
    return map;
  }

  @override
  String toString() {
    return (StringBuffer('KalamCachedRowsCompanion(')
          ..write('accountKey: $accountKey, ')
          ..write('tableId: $tableId, ')
          ..write('rowKey: $rowKey, ')
          ..write('valuesJson: $valuesJson, ')
          ..write('updatedAt: $updatedAt, ')
          ..write('rowid: $rowid')
          ..write(')'))
        .toString();
  }
}

class $KalamActionStepsTable extends KalamActionSteps
    with TableInfo<$KalamActionStepsTable, StoredActionStep> {
  @override
  final GeneratedDatabase attachedDatabase;
  final String? _alias;
  $KalamActionStepsTable(this.attachedDatabase, [this._alias]);
  static const VerificationMeta _actionIdMeta = const VerificationMeta(
    'actionId',
  );
  @override
  late final GeneratedColumn<String> actionId = GeneratedColumn<String>(
    'action_id',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _nameMeta = const VerificationMeta('name');
  @override
  late final GeneratedColumn<String> name = GeneratedColumn<String>(
    'name',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _statusMeta = const VerificationMeta('status');
  @override
  late final GeneratedColumn<String> status = GeneratedColumn<String>(
    'status',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _resultJsonMeta = const VerificationMeta(
    'resultJson',
  );
  @override
  late final GeneratedColumn<String> resultJson = GeneratedColumn<String>(
    'result_json',
    aliasedName,
    true,
    type: DriftSqlType.string,
    requiredDuringInsert: false,
  );
  static const VerificationMeta _lastErrorMeta = const VerificationMeta(
    'lastError',
  );
  @override
  late final GeneratedColumn<String> lastError = GeneratedColumn<String>(
    'last_error',
    aliasedName,
    true,
    type: DriftSqlType.string,
    requiredDuringInsert: false,
  );
  static const VerificationMeta _updatedAtMeta = const VerificationMeta(
    'updatedAt',
  );
  @override
  late final GeneratedColumn<DateTime> updatedAt = GeneratedColumn<DateTime>(
    'updated_at',
    aliasedName,
    false,
    type: DriftSqlType.dateTime,
    requiredDuringInsert: true,
  );
  @override
  List<GeneratedColumn> get $columns => [
    actionId,
    name,
    status,
    resultJson,
    lastError,
    updatedAt,
  ];
  @override
  String get aliasedName => _alias ?? actualTableName;
  @override
  String get actualTableName => $name;
  static const String $name = 'kalam_action_steps';
  @override
  VerificationContext validateIntegrity(
    Insertable<StoredActionStep> instance, {
    bool isInserting = false,
  }) {
    final context = VerificationContext();
    final data = instance.toColumns(true);
    if (data.containsKey('action_id')) {
      context.handle(
        _actionIdMeta,
        actionId.isAcceptableOrUnknown(data['action_id']!, _actionIdMeta),
      );
    } else if (isInserting) {
      context.missing(_actionIdMeta);
    }
    if (data.containsKey('name')) {
      context.handle(
        _nameMeta,
        name.isAcceptableOrUnknown(data['name']!, _nameMeta),
      );
    } else if (isInserting) {
      context.missing(_nameMeta);
    }
    if (data.containsKey('status')) {
      context.handle(
        _statusMeta,
        status.isAcceptableOrUnknown(data['status']!, _statusMeta),
      );
    } else if (isInserting) {
      context.missing(_statusMeta);
    }
    if (data.containsKey('result_json')) {
      context.handle(
        _resultJsonMeta,
        resultJson.isAcceptableOrUnknown(data['result_json']!, _resultJsonMeta),
      );
    }
    if (data.containsKey('last_error')) {
      context.handle(
        _lastErrorMeta,
        lastError.isAcceptableOrUnknown(data['last_error']!, _lastErrorMeta),
      );
    }
    if (data.containsKey('updated_at')) {
      context.handle(
        _updatedAtMeta,
        updatedAt.isAcceptableOrUnknown(data['updated_at']!, _updatedAtMeta),
      );
    } else if (isInserting) {
      context.missing(_updatedAtMeta);
    }
    return context;
  }

  @override
  Set<GeneratedColumn> get $primaryKey => {actionId, name};
  @override
  StoredActionStep map(Map<String, dynamic> data, {String? tablePrefix}) {
    final effectivePrefix = tablePrefix != null ? '$tablePrefix.' : '';
    return StoredActionStep(
      actionId: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}action_id'],
      )!,
      name: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}name'],
      )!,
      status: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}status'],
      )!,
      resultJson: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}result_json'],
      ),
      lastError: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}last_error'],
      ),
      updatedAt: attachedDatabase.typeMapping.read(
        DriftSqlType.dateTime,
        data['${effectivePrefix}updated_at'],
      )!,
    );
  }

  @override
  $KalamActionStepsTable createAlias(String alias) {
    return $KalamActionStepsTable(attachedDatabase, alias);
  }
}

class StoredActionStep extends DataClass
    implements Insertable<StoredActionStep> {
  final String actionId;
  final String name;
  final String status;
  final String? resultJson;
  final String? lastError;
  final DateTime updatedAt;
  const StoredActionStep({
    required this.actionId,
    required this.name,
    required this.status,
    this.resultJson,
    this.lastError,
    required this.updatedAt,
  });
  @override
  Map<String, Expression> toColumns(bool nullToAbsent) {
    final map = <String, Expression>{};
    map['action_id'] = Variable<String>(actionId);
    map['name'] = Variable<String>(name);
    map['status'] = Variable<String>(status);
    if (!nullToAbsent || resultJson != null) {
      map['result_json'] = Variable<String>(resultJson);
    }
    if (!nullToAbsent || lastError != null) {
      map['last_error'] = Variable<String>(lastError);
    }
    map['updated_at'] = Variable<DateTime>(updatedAt);
    return map;
  }

  KalamActionStepsCompanion toCompanion(bool nullToAbsent) {
    return KalamActionStepsCompanion(
      actionId: Value(actionId),
      name: Value(name),
      status: Value(status),
      resultJson: resultJson == null && nullToAbsent
          ? const Value.absent()
          : Value(resultJson),
      lastError: lastError == null && nullToAbsent
          ? const Value.absent()
          : Value(lastError),
      updatedAt: Value(updatedAt),
    );
  }

  factory StoredActionStep.fromJson(
    Map<String, dynamic> json, {
    ValueSerializer? serializer,
  }) {
    serializer ??= driftRuntimeOptions.defaultSerializer;
    return StoredActionStep(
      actionId: serializer.fromJson<String>(json['actionId']),
      name: serializer.fromJson<String>(json['name']),
      status: serializer.fromJson<String>(json['status']),
      resultJson: serializer.fromJson<String?>(json['resultJson']),
      lastError: serializer.fromJson<String?>(json['lastError']),
      updatedAt: serializer.fromJson<DateTime>(json['updatedAt']),
    );
  }
  @override
  Map<String, dynamic> toJson({ValueSerializer? serializer}) {
    serializer ??= driftRuntimeOptions.defaultSerializer;
    return <String, dynamic>{
      'actionId': serializer.toJson<String>(actionId),
      'name': serializer.toJson<String>(name),
      'status': serializer.toJson<String>(status),
      'resultJson': serializer.toJson<String?>(resultJson),
      'lastError': serializer.toJson<String?>(lastError),
      'updatedAt': serializer.toJson<DateTime>(updatedAt),
    };
  }

  StoredActionStep copyWith({
    String? actionId,
    String? name,
    String? status,
    Value<String?> resultJson = const Value.absent(),
    Value<String?> lastError = const Value.absent(),
    DateTime? updatedAt,
  }) => StoredActionStep(
    actionId: actionId ?? this.actionId,
    name: name ?? this.name,
    status: status ?? this.status,
    resultJson: resultJson.present ? resultJson.value : this.resultJson,
    lastError: lastError.present ? lastError.value : this.lastError,
    updatedAt: updatedAt ?? this.updatedAt,
  );
  StoredActionStep copyWithCompanion(KalamActionStepsCompanion data) {
    return StoredActionStep(
      actionId: data.actionId.present ? data.actionId.value : this.actionId,
      name: data.name.present ? data.name.value : this.name,
      status: data.status.present ? data.status.value : this.status,
      resultJson: data.resultJson.present
          ? data.resultJson.value
          : this.resultJson,
      lastError: data.lastError.present ? data.lastError.value : this.lastError,
      updatedAt: data.updatedAt.present ? data.updatedAt.value : this.updatedAt,
    );
  }

  @override
  String toString() {
    return (StringBuffer('StoredActionStep(')
          ..write('actionId: $actionId, ')
          ..write('name: $name, ')
          ..write('status: $status, ')
          ..write('resultJson: $resultJson, ')
          ..write('lastError: $lastError, ')
          ..write('updatedAt: $updatedAt')
          ..write(')'))
        .toString();
  }

  @override
  int get hashCode =>
      Object.hash(actionId, name, status, resultJson, lastError, updatedAt);
  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      (other is StoredActionStep &&
          other.actionId == this.actionId &&
          other.name == this.name &&
          other.status == this.status &&
          other.resultJson == this.resultJson &&
          other.lastError == this.lastError &&
          other.updatedAt == this.updatedAt);
}

class KalamActionStepsCompanion extends UpdateCompanion<StoredActionStep> {
  final Value<String> actionId;
  final Value<String> name;
  final Value<String> status;
  final Value<String?> resultJson;
  final Value<String?> lastError;
  final Value<DateTime> updatedAt;
  final Value<int> rowid;
  const KalamActionStepsCompanion({
    this.actionId = const Value.absent(),
    this.name = const Value.absent(),
    this.status = const Value.absent(),
    this.resultJson = const Value.absent(),
    this.lastError = const Value.absent(),
    this.updatedAt = const Value.absent(),
    this.rowid = const Value.absent(),
  });
  KalamActionStepsCompanion.insert({
    required String actionId,
    required String name,
    required String status,
    this.resultJson = const Value.absent(),
    this.lastError = const Value.absent(),
    required DateTime updatedAt,
    this.rowid = const Value.absent(),
  }) : actionId = Value(actionId),
       name = Value(name),
       status = Value(status),
       updatedAt = Value(updatedAt);
  static Insertable<StoredActionStep> custom({
    Expression<String>? actionId,
    Expression<String>? name,
    Expression<String>? status,
    Expression<String>? resultJson,
    Expression<String>? lastError,
    Expression<DateTime>? updatedAt,
    Expression<int>? rowid,
  }) {
    return RawValuesInsertable({
      if (actionId != null) 'action_id': actionId,
      if (name != null) 'name': name,
      if (status != null) 'status': status,
      if (resultJson != null) 'result_json': resultJson,
      if (lastError != null) 'last_error': lastError,
      if (updatedAt != null) 'updated_at': updatedAt,
      if (rowid != null) 'rowid': rowid,
    });
  }

  KalamActionStepsCompanion copyWith({
    Value<String>? actionId,
    Value<String>? name,
    Value<String>? status,
    Value<String?>? resultJson,
    Value<String?>? lastError,
    Value<DateTime>? updatedAt,
    Value<int>? rowid,
  }) {
    return KalamActionStepsCompanion(
      actionId: actionId ?? this.actionId,
      name: name ?? this.name,
      status: status ?? this.status,
      resultJson: resultJson ?? this.resultJson,
      lastError: lastError ?? this.lastError,
      updatedAt: updatedAt ?? this.updatedAt,
      rowid: rowid ?? this.rowid,
    );
  }

  @override
  Map<String, Expression> toColumns(bool nullToAbsent) {
    final map = <String, Expression>{};
    if (actionId.present) {
      map['action_id'] = Variable<String>(actionId.value);
    }
    if (name.present) {
      map['name'] = Variable<String>(name.value);
    }
    if (status.present) {
      map['status'] = Variable<String>(status.value);
    }
    if (resultJson.present) {
      map['result_json'] = Variable<String>(resultJson.value);
    }
    if (lastError.present) {
      map['last_error'] = Variable<String>(lastError.value);
    }
    if (updatedAt.present) {
      map['updated_at'] = Variable<DateTime>(updatedAt.value);
    }
    if (rowid.present) {
      map['rowid'] = Variable<int>(rowid.value);
    }
    return map;
  }

  @override
  String toString() {
    return (StringBuffer('KalamActionStepsCompanion(')
          ..write('actionId: $actionId, ')
          ..write('name: $name, ')
          ..write('status: $status, ')
          ..write('resultJson: $resultJson, ')
          ..write('lastError: $lastError, ')
          ..write('updatedAt: $updatedAt, ')
          ..write('rowid: $rowid')
          ..write(')'))
        .toString();
  }
}

class $KalamCheckpointsTable extends KalamCheckpoints
    with TableInfo<$KalamCheckpointsTable, StoredCheckpoint> {
  @override
  final GeneratedDatabase attachedDatabase;
  final String? _alias;
  $KalamCheckpointsTable(this.attachedDatabase, [this._alias]);
  static const VerificationMeta _accountKeyMeta = const VerificationMeta(
    'accountKey',
  );
  @override
  late final GeneratedColumn<String> accountKey = GeneratedColumn<String>(
    'account_key',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _subscriptionIdMeta = const VerificationMeta(
    'subscriptionId',
  );
  @override
  late final GeneratedColumn<String> subscriptionId = GeneratedColumn<String>(
    'subscription_id',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _seqMeta = const VerificationMeta('seq');
  @override
  late final GeneratedColumn<String> seq = GeneratedColumn<String>(
    'seq',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _updatedAtMeta = const VerificationMeta(
    'updatedAt',
  );
  @override
  late final GeneratedColumn<DateTime> updatedAt = GeneratedColumn<DateTime>(
    'updated_at',
    aliasedName,
    false,
    type: DriftSqlType.dateTime,
    requiredDuringInsert: true,
  );
  @override
  List<GeneratedColumn> get $columns => [
    accountKey,
    subscriptionId,
    seq,
    updatedAt,
  ];
  @override
  String get aliasedName => _alias ?? actualTableName;
  @override
  String get actualTableName => $name;
  static const String $name = 'kalam_checkpoints';
  @override
  VerificationContext validateIntegrity(
    Insertable<StoredCheckpoint> instance, {
    bool isInserting = false,
  }) {
    final context = VerificationContext();
    final data = instance.toColumns(true);
    if (data.containsKey('account_key')) {
      context.handle(
        _accountKeyMeta,
        accountKey.isAcceptableOrUnknown(data['account_key']!, _accountKeyMeta),
      );
    } else if (isInserting) {
      context.missing(_accountKeyMeta);
    }
    if (data.containsKey('subscription_id')) {
      context.handle(
        _subscriptionIdMeta,
        subscriptionId.isAcceptableOrUnknown(
          data['subscription_id']!,
          _subscriptionIdMeta,
        ),
      );
    } else if (isInserting) {
      context.missing(_subscriptionIdMeta);
    }
    if (data.containsKey('seq')) {
      context.handle(
        _seqMeta,
        seq.isAcceptableOrUnknown(data['seq']!, _seqMeta),
      );
    } else if (isInserting) {
      context.missing(_seqMeta);
    }
    if (data.containsKey('updated_at')) {
      context.handle(
        _updatedAtMeta,
        updatedAt.isAcceptableOrUnknown(data['updated_at']!, _updatedAtMeta),
      );
    } else if (isInserting) {
      context.missing(_updatedAtMeta);
    }
    return context;
  }

  @override
  Set<GeneratedColumn> get $primaryKey => {accountKey, subscriptionId};
  @override
  StoredCheckpoint map(Map<String, dynamic> data, {String? tablePrefix}) {
    final effectivePrefix = tablePrefix != null ? '$tablePrefix.' : '';
    return StoredCheckpoint(
      accountKey: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}account_key'],
      )!,
      subscriptionId: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}subscription_id'],
      )!,
      seq: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}seq'],
      )!,
      updatedAt: attachedDatabase.typeMapping.read(
        DriftSqlType.dateTime,
        data['${effectivePrefix}updated_at'],
      )!,
    );
  }

  @override
  $KalamCheckpointsTable createAlias(String alias) {
    return $KalamCheckpointsTable(attachedDatabase, alias);
  }
}

class StoredCheckpoint extends DataClass
    implements Insertable<StoredCheckpoint> {
  final String accountKey;
  final String subscriptionId;
  final String seq;
  final DateTime updatedAt;
  const StoredCheckpoint({
    required this.accountKey,
    required this.subscriptionId,
    required this.seq,
    required this.updatedAt,
  });
  @override
  Map<String, Expression> toColumns(bool nullToAbsent) {
    final map = <String, Expression>{};
    map['account_key'] = Variable<String>(accountKey);
    map['subscription_id'] = Variable<String>(subscriptionId);
    map['seq'] = Variable<String>(seq);
    map['updated_at'] = Variable<DateTime>(updatedAt);
    return map;
  }

  KalamCheckpointsCompanion toCompanion(bool nullToAbsent) {
    return KalamCheckpointsCompanion(
      accountKey: Value(accountKey),
      subscriptionId: Value(subscriptionId),
      seq: Value(seq),
      updatedAt: Value(updatedAt),
    );
  }

  factory StoredCheckpoint.fromJson(
    Map<String, dynamic> json, {
    ValueSerializer? serializer,
  }) {
    serializer ??= driftRuntimeOptions.defaultSerializer;
    return StoredCheckpoint(
      accountKey: serializer.fromJson<String>(json['accountKey']),
      subscriptionId: serializer.fromJson<String>(json['subscriptionId']),
      seq: serializer.fromJson<String>(json['seq']),
      updatedAt: serializer.fromJson<DateTime>(json['updatedAt']),
    );
  }
  @override
  Map<String, dynamic> toJson({ValueSerializer? serializer}) {
    serializer ??= driftRuntimeOptions.defaultSerializer;
    return <String, dynamic>{
      'accountKey': serializer.toJson<String>(accountKey),
      'subscriptionId': serializer.toJson<String>(subscriptionId),
      'seq': serializer.toJson<String>(seq),
      'updatedAt': serializer.toJson<DateTime>(updatedAt),
    };
  }

  StoredCheckpoint copyWith({
    String? accountKey,
    String? subscriptionId,
    String? seq,
    DateTime? updatedAt,
  }) => StoredCheckpoint(
    accountKey: accountKey ?? this.accountKey,
    subscriptionId: subscriptionId ?? this.subscriptionId,
    seq: seq ?? this.seq,
    updatedAt: updatedAt ?? this.updatedAt,
  );
  StoredCheckpoint copyWithCompanion(KalamCheckpointsCompanion data) {
    return StoredCheckpoint(
      accountKey: data.accountKey.present
          ? data.accountKey.value
          : this.accountKey,
      subscriptionId: data.subscriptionId.present
          ? data.subscriptionId.value
          : this.subscriptionId,
      seq: data.seq.present ? data.seq.value : this.seq,
      updatedAt: data.updatedAt.present ? data.updatedAt.value : this.updatedAt,
    );
  }

  @override
  String toString() {
    return (StringBuffer('StoredCheckpoint(')
          ..write('accountKey: $accountKey, ')
          ..write('subscriptionId: $subscriptionId, ')
          ..write('seq: $seq, ')
          ..write('updatedAt: $updatedAt')
          ..write(')'))
        .toString();
  }

  @override
  int get hashCode => Object.hash(accountKey, subscriptionId, seq, updatedAt);
  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      (other is StoredCheckpoint &&
          other.accountKey == this.accountKey &&
          other.subscriptionId == this.subscriptionId &&
          other.seq == this.seq &&
          other.updatedAt == this.updatedAt);
}

class KalamCheckpointsCompanion extends UpdateCompanion<StoredCheckpoint> {
  final Value<String> accountKey;
  final Value<String> subscriptionId;
  final Value<String> seq;
  final Value<DateTime> updatedAt;
  final Value<int> rowid;
  const KalamCheckpointsCompanion({
    this.accountKey = const Value.absent(),
    this.subscriptionId = const Value.absent(),
    this.seq = const Value.absent(),
    this.updatedAt = const Value.absent(),
    this.rowid = const Value.absent(),
  });
  KalamCheckpointsCompanion.insert({
    required String accountKey,
    required String subscriptionId,
    required String seq,
    required DateTime updatedAt,
    this.rowid = const Value.absent(),
  }) : accountKey = Value(accountKey),
       subscriptionId = Value(subscriptionId),
       seq = Value(seq),
       updatedAt = Value(updatedAt);
  static Insertable<StoredCheckpoint> custom({
    Expression<String>? accountKey,
    Expression<String>? subscriptionId,
    Expression<String>? seq,
    Expression<DateTime>? updatedAt,
    Expression<int>? rowid,
  }) {
    return RawValuesInsertable({
      if (accountKey != null) 'account_key': accountKey,
      if (subscriptionId != null) 'subscription_id': subscriptionId,
      if (seq != null) 'seq': seq,
      if (updatedAt != null) 'updated_at': updatedAt,
      if (rowid != null) 'rowid': rowid,
    });
  }

  KalamCheckpointsCompanion copyWith({
    Value<String>? accountKey,
    Value<String>? subscriptionId,
    Value<String>? seq,
    Value<DateTime>? updatedAt,
    Value<int>? rowid,
  }) {
    return KalamCheckpointsCompanion(
      accountKey: accountKey ?? this.accountKey,
      subscriptionId: subscriptionId ?? this.subscriptionId,
      seq: seq ?? this.seq,
      updatedAt: updatedAt ?? this.updatedAt,
      rowid: rowid ?? this.rowid,
    );
  }

  @override
  Map<String, Expression> toColumns(bool nullToAbsent) {
    final map = <String, Expression>{};
    if (accountKey.present) {
      map['account_key'] = Variable<String>(accountKey.value);
    }
    if (subscriptionId.present) {
      map['subscription_id'] = Variable<String>(subscriptionId.value);
    }
    if (seq.present) {
      map['seq'] = Variable<String>(seq.value);
    }
    if (updatedAt.present) {
      map['updated_at'] = Variable<DateTime>(updatedAt.value);
    }
    if (rowid.present) {
      map['rowid'] = Variable<int>(rowid.value);
    }
    return map;
  }

  @override
  String toString() {
    return (StringBuffer('KalamCheckpointsCompanion(')
          ..write('accountKey: $accountKey, ')
          ..write('subscriptionId: $subscriptionId, ')
          ..write('seq: $seq, ')
          ..write('updatedAt: $updatedAt, ')
          ..write('rowid: $rowid')
          ..write(')'))
        .toString();
  }
}

class $KalamRowStatesTable extends KalamRowStates
    with TableInfo<$KalamRowStatesTable, StoredRowState> {
  @override
  final GeneratedDatabase attachedDatabase;
  final String? _alias;
  $KalamRowStatesTable(this.attachedDatabase, [this._alias]);
  static const VerificationMeta _accountKeyMeta = const VerificationMeta(
    'accountKey',
  );
  @override
  late final GeneratedColumn<String> accountKey = GeneratedColumn<String>(
    'account_key',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _tableIdMeta = const VerificationMeta(
    'tableId',
  );
  @override
  late final GeneratedColumn<String> tableId = GeneratedColumn<String>(
    'table_id',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _rowKeyMeta = const VerificationMeta('rowKey');
  @override
  late final GeneratedColumn<String> rowKey = GeneratedColumn<String>(
    'row_key',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _phaseMeta = const VerificationMeta('phase');
  @override
  late final GeneratedColumn<String> phase = GeneratedColumn<String>(
    'phase',
    aliasedName,
    false,
    type: DriftSqlType.string,
    requiredDuringInsert: true,
  );
  static const VerificationMeta _actionIdMeta = const VerificationMeta(
    'actionId',
  );
  @override
  late final GeneratedColumn<String> actionId = GeneratedColumn<String>(
    'action_id',
    aliasedName,
    true,
    type: DriftSqlType.string,
    requiredDuringInsert: false,
  );
  static const VerificationMeta _attemptCountMeta = const VerificationMeta(
    'attemptCount',
  );
  @override
  late final GeneratedColumn<int> attemptCount = GeneratedColumn<int>(
    'attempt_count',
    aliasedName,
    false,
    type: DriftSqlType.int,
    requiredDuringInsert: false,
    defaultValue: const Constant(0),
  );
  static const VerificationMeta _nextRetryAtMeta = const VerificationMeta(
    'nextRetryAt',
  );
  @override
  late final GeneratedColumn<DateTime> nextRetryAt = GeneratedColumn<DateTime>(
    'next_retry_at',
    aliasedName,
    true,
    type: DriftSqlType.dateTime,
    requiredDuringInsert: false,
  );
  static const VerificationMeta _errorCodeMeta = const VerificationMeta(
    'errorCode',
  );
  @override
  late final GeneratedColumn<String> errorCode = GeneratedColumn<String>(
    'error_code',
    aliasedName,
    true,
    type: DriftSqlType.string,
    requiredDuringInsert: false,
  );
  static const VerificationMeta _errorMessageMeta = const VerificationMeta(
    'errorMessage',
  );
  @override
  late final GeneratedColumn<String> errorMessage = GeneratedColumn<String>(
    'error_message',
    aliasedName,
    true,
    type: DriftSqlType.string,
    requiredDuringInsert: false,
  );
  static const VerificationMeta _lastServerSeqMeta = const VerificationMeta(
    'lastServerSeq',
  );
  @override
  late final GeneratedColumn<String> lastServerSeq = GeneratedColumn<String>(
    'last_server_seq',
    aliasedName,
    true,
    type: DriftSqlType.string,
    requiredDuringInsert: false,
  );
  static const VerificationMeta _pendingValuesJsonMeta = const VerificationMeta(
    'pendingValuesJson',
  );
  @override
  late final GeneratedColumn<String> pendingValuesJson =
      GeneratedColumn<String>(
        'pending_values_json',
        aliasedName,
        true,
        type: DriftSqlType.string,
        requiredDuringInsert: false,
      );
  static const VerificationMeta _tombstoneMeta = const VerificationMeta(
    'tombstone',
  );
  @override
  late final GeneratedColumn<bool> tombstone = GeneratedColumn<bool>(
    'tombstone',
    aliasedName,
    false,
    type: DriftSqlType.bool,
    requiredDuringInsert: false,
    defaultConstraints: GeneratedColumn.constraintIsAlways(
      'CHECK ("tombstone" IN (0, 1))',
    ),
    defaultValue: const Constant(false),
  );
  static const VerificationMeta _updatedAtMeta = const VerificationMeta(
    'updatedAt',
  );
  @override
  late final GeneratedColumn<DateTime> updatedAt = GeneratedColumn<DateTime>(
    'updated_at',
    aliasedName,
    false,
    type: DriftSqlType.dateTime,
    requiredDuringInsert: true,
  );
  @override
  List<GeneratedColumn> get $columns => [
    accountKey,
    tableId,
    rowKey,
    phase,
    actionId,
    attemptCount,
    nextRetryAt,
    errorCode,
    errorMessage,
    lastServerSeq,
    pendingValuesJson,
    tombstone,
    updatedAt,
  ];
  @override
  String get aliasedName => _alias ?? actualTableName;
  @override
  String get actualTableName => $name;
  static const String $name = 'kalam_row_states';
  @override
  VerificationContext validateIntegrity(
    Insertable<StoredRowState> instance, {
    bool isInserting = false,
  }) {
    final context = VerificationContext();
    final data = instance.toColumns(true);
    if (data.containsKey('account_key')) {
      context.handle(
        _accountKeyMeta,
        accountKey.isAcceptableOrUnknown(data['account_key']!, _accountKeyMeta),
      );
    } else if (isInserting) {
      context.missing(_accountKeyMeta);
    }
    if (data.containsKey('table_id')) {
      context.handle(
        _tableIdMeta,
        tableId.isAcceptableOrUnknown(data['table_id']!, _tableIdMeta),
      );
    } else if (isInserting) {
      context.missing(_tableIdMeta);
    }
    if (data.containsKey('row_key')) {
      context.handle(
        _rowKeyMeta,
        rowKey.isAcceptableOrUnknown(data['row_key']!, _rowKeyMeta),
      );
    } else if (isInserting) {
      context.missing(_rowKeyMeta);
    }
    if (data.containsKey('phase')) {
      context.handle(
        _phaseMeta,
        phase.isAcceptableOrUnknown(data['phase']!, _phaseMeta),
      );
    } else if (isInserting) {
      context.missing(_phaseMeta);
    }
    if (data.containsKey('action_id')) {
      context.handle(
        _actionIdMeta,
        actionId.isAcceptableOrUnknown(data['action_id']!, _actionIdMeta),
      );
    }
    if (data.containsKey('attempt_count')) {
      context.handle(
        _attemptCountMeta,
        attemptCount.isAcceptableOrUnknown(
          data['attempt_count']!,
          _attemptCountMeta,
        ),
      );
    }
    if (data.containsKey('next_retry_at')) {
      context.handle(
        _nextRetryAtMeta,
        nextRetryAt.isAcceptableOrUnknown(
          data['next_retry_at']!,
          _nextRetryAtMeta,
        ),
      );
    }
    if (data.containsKey('error_code')) {
      context.handle(
        _errorCodeMeta,
        errorCode.isAcceptableOrUnknown(data['error_code']!, _errorCodeMeta),
      );
    }
    if (data.containsKey('error_message')) {
      context.handle(
        _errorMessageMeta,
        errorMessage.isAcceptableOrUnknown(
          data['error_message']!,
          _errorMessageMeta,
        ),
      );
    }
    if (data.containsKey('last_server_seq')) {
      context.handle(
        _lastServerSeqMeta,
        lastServerSeq.isAcceptableOrUnknown(
          data['last_server_seq']!,
          _lastServerSeqMeta,
        ),
      );
    }
    if (data.containsKey('pending_values_json')) {
      context.handle(
        _pendingValuesJsonMeta,
        pendingValuesJson.isAcceptableOrUnknown(
          data['pending_values_json']!,
          _pendingValuesJsonMeta,
        ),
      );
    }
    if (data.containsKey('tombstone')) {
      context.handle(
        _tombstoneMeta,
        tombstone.isAcceptableOrUnknown(data['tombstone']!, _tombstoneMeta),
      );
    }
    if (data.containsKey('updated_at')) {
      context.handle(
        _updatedAtMeta,
        updatedAt.isAcceptableOrUnknown(data['updated_at']!, _updatedAtMeta),
      );
    } else if (isInserting) {
      context.missing(_updatedAtMeta);
    }
    return context;
  }

  @override
  Set<GeneratedColumn> get $primaryKey => {accountKey, tableId, rowKey};
  @override
  StoredRowState map(Map<String, dynamic> data, {String? tablePrefix}) {
    final effectivePrefix = tablePrefix != null ? '$tablePrefix.' : '';
    return StoredRowState(
      accountKey: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}account_key'],
      )!,
      tableId: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}table_id'],
      )!,
      rowKey: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}row_key'],
      )!,
      phase: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}phase'],
      )!,
      actionId: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}action_id'],
      ),
      attemptCount: attachedDatabase.typeMapping.read(
        DriftSqlType.int,
        data['${effectivePrefix}attempt_count'],
      )!,
      nextRetryAt: attachedDatabase.typeMapping.read(
        DriftSqlType.dateTime,
        data['${effectivePrefix}next_retry_at'],
      ),
      errorCode: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}error_code'],
      ),
      errorMessage: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}error_message'],
      ),
      lastServerSeq: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}last_server_seq'],
      ),
      pendingValuesJson: attachedDatabase.typeMapping.read(
        DriftSqlType.string,
        data['${effectivePrefix}pending_values_json'],
      ),
      tombstone: attachedDatabase.typeMapping.read(
        DriftSqlType.bool,
        data['${effectivePrefix}tombstone'],
      )!,
      updatedAt: attachedDatabase.typeMapping.read(
        DriftSqlType.dateTime,
        data['${effectivePrefix}updated_at'],
      )!,
    );
  }

  @override
  $KalamRowStatesTable createAlias(String alias) {
    return $KalamRowStatesTable(attachedDatabase, alias);
  }
}

class StoredRowState extends DataClass implements Insertable<StoredRowState> {
  final String accountKey;
  final String tableId;
  final String rowKey;
  final String phase;
  final String? actionId;
  final int attemptCount;
  final DateTime? nextRetryAt;
  final String? errorCode;
  final String? errorMessage;
  final String? lastServerSeq;
  final String? pendingValuesJson;
  final bool tombstone;
  final DateTime updatedAt;
  const StoredRowState({
    required this.accountKey,
    required this.tableId,
    required this.rowKey,
    required this.phase,
    this.actionId,
    required this.attemptCount,
    this.nextRetryAt,
    this.errorCode,
    this.errorMessage,
    this.lastServerSeq,
    this.pendingValuesJson,
    required this.tombstone,
    required this.updatedAt,
  });
  @override
  Map<String, Expression> toColumns(bool nullToAbsent) {
    final map = <String, Expression>{};
    map['account_key'] = Variable<String>(accountKey);
    map['table_id'] = Variable<String>(tableId);
    map['row_key'] = Variable<String>(rowKey);
    map['phase'] = Variable<String>(phase);
    if (!nullToAbsent || actionId != null) {
      map['action_id'] = Variable<String>(actionId);
    }
    map['attempt_count'] = Variable<int>(attemptCount);
    if (!nullToAbsent || nextRetryAt != null) {
      map['next_retry_at'] = Variable<DateTime>(nextRetryAt);
    }
    if (!nullToAbsent || errorCode != null) {
      map['error_code'] = Variable<String>(errorCode);
    }
    if (!nullToAbsent || errorMessage != null) {
      map['error_message'] = Variable<String>(errorMessage);
    }
    if (!nullToAbsent || lastServerSeq != null) {
      map['last_server_seq'] = Variable<String>(lastServerSeq);
    }
    if (!nullToAbsent || pendingValuesJson != null) {
      map['pending_values_json'] = Variable<String>(pendingValuesJson);
    }
    map['tombstone'] = Variable<bool>(tombstone);
    map['updated_at'] = Variable<DateTime>(updatedAt);
    return map;
  }

  KalamRowStatesCompanion toCompanion(bool nullToAbsent) {
    return KalamRowStatesCompanion(
      accountKey: Value(accountKey),
      tableId: Value(tableId),
      rowKey: Value(rowKey),
      phase: Value(phase),
      actionId: actionId == null && nullToAbsent
          ? const Value.absent()
          : Value(actionId),
      attemptCount: Value(attemptCount),
      nextRetryAt: nextRetryAt == null && nullToAbsent
          ? const Value.absent()
          : Value(nextRetryAt),
      errorCode: errorCode == null && nullToAbsent
          ? const Value.absent()
          : Value(errorCode),
      errorMessage: errorMessage == null && nullToAbsent
          ? const Value.absent()
          : Value(errorMessage),
      lastServerSeq: lastServerSeq == null && nullToAbsent
          ? const Value.absent()
          : Value(lastServerSeq),
      pendingValuesJson: pendingValuesJson == null && nullToAbsent
          ? const Value.absent()
          : Value(pendingValuesJson),
      tombstone: Value(tombstone),
      updatedAt: Value(updatedAt),
    );
  }

  factory StoredRowState.fromJson(
    Map<String, dynamic> json, {
    ValueSerializer? serializer,
  }) {
    serializer ??= driftRuntimeOptions.defaultSerializer;
    return StoredRowState(
      accountKey: serializer.fromJson<String>(json['accountKey']),
      tableId: serializer.fromJson<String>(json['tableId']),
      rowKey: serializer.fromJson<String>(json['rowKey']),
      phase: serializer.fromJson<String>(json['phase']),
      actionId: serializer.fromJson<String?>(json['actionId']),
      attemptCount: serializer.fromJson<int>(json['attemptCount']),
      nextRetryAt: serializer.fromJson<DateTime?>(json['nextRetryAt']),
      errorCode: serializer.fromJson<String?>(json['errorCode']),
      errorMessage: serializer.fromJson<String?>(json['errorMessage']),
      lastServerSeq: serializer.fromJson<String?>(json['lastServerSeq']),
      pendingValuesJson: serializer.fromJson<String?>(
        json['pendingValuesJson'],
      ),
      tombstone: serializer.fromJson<bool>(json['tombstone']),
      updatedAt: serializer.fromJson<DateTime>(json['updatedAt']),
    );
  }
  @override
  Map<String, dynamic> toJson({ValueSerializer? serializer}) {
    serializer ??= driftRuntimeOptions.defaultSerializer;
    return <String, dynamic>{
      'accountKey': serializer.toJson<String>(accountKey),
      'tableId': serializer.toJson<String>(tableId),
      'rowKey': serializer.toJson<String>(rowKey),
      'phase': serializer.toJson<String>(phase),
      'actionId': serializer.toJson<String?>(actionId),
      'attemptCount': serializer.toJson<int>(attemptCount),
      'nextRetryAt': serializer.toJson<DateTime?>(nextRetryAt),
      'errorCode': serializer.toJson<String?>(errorCode),
      'errorMessage': serializer.toJson<String?>(errorMessage),
      'lastServerSeq': serializer.toJson<String?>(lastServerSeq),
      'pendingValuesJson': serializer.toJson<String?>(pendingValuesJson),
      'tombstone': serializer.toJson<bool>(tombstone),
      'updatedAt': serializer.toJson<DateTime>(updatedAt),
    };
  }

  StoredRowState copyWith({
    String? accountKey,
    String? tableId,
    String? rowKey,
    String? phase,
    Value<String?> actionId = const Value.absent(),
    int? attemptCount,
    Value<DateTime?> nextRetryAt = const Value.absent(),
    Value<String?> errorCode = const Value.absent(),
    Value<String?> errorMessage = const Value.absent(),
    Value<String?> lastServerSeq = const Value.absent(),
    Value<String?> pendingValuesJson = const Value.absent(),
    bool? tombstone,
    DateTime? updatedAt,
  }) => StoredRowState(
    accountKey: accountKey ?? this.accountKey,
    tableId: tableId ?? this.tableId,
    rowKey: rowKey ?? this.rowKey,
    phase: phase ?? this.phase,
    actionId: actionId.present ? actionId.value : this.actionId,
    attemptCount: attemptCount ?? this.attemptCount,
    nextRetryAt: nextRetryAt.present ? nextRetryAt.value : this.nextRetryAt,
    errorCode: errorCode.present ? errorCode.value : this.errorCode,
    errorMessage: errorMessage.present ? errorMessage.value : this.errorMessage,
    lastServerSeq: lastServerSeq.present
        ? lastServerSeq.value
        : this.lastServerSeq,
    pendingValuesJson: pendingValuesJson.present
        ? pendingValuesJson.value
        : this.pendingValuesJson,
    tombstone: tombstone ?? this.tombstone,
    updatedAt: updatedAt ?? this.updatedAt,
  );
  StoredRowState copyWithCompanion(KalamRowStatesCompanion data) {
    return StoredRowState(
      accountKey: data.accountKey.present
          ? data.accountKey.value
          : this.accountKey,
      tableId: data.tableId.present ? data.tableId.value : this.tableId,
      rowKey: data.rowKey.present ? data.rowKey.value : this.rowKey,
      phase: data.phase.present ? data.phase.value : this.phase,
      actionId: data.actionId.present ? data.actionId.value : this.actionId,
      attemptCount: data.attemptCount.present
          ? data.attemptCount.value
          : this.attemptCount,
      nextRetryAt: data.nextRetryAt.present
          ? data.nextRetryAt.value
          : this.nextRetryAt,
      errorCode: data.errorCode.present ? data.errorCode.value : this.errorCode,
      errorMessage: data.errorMessage.present
          ? data.errorMessage.value
          : this.errorMessage,
      lastServerSeq: data.lastServerSeq.present
          ? data.lastServerSeq.value
          : this.lastServerSeq,
      pendingValuesJson: data.pendingValuesJson.present
          ? data.pendingValuesJson.value
          : this.pendingValuesJson,
      tombstone: data.tombstone.present ? data.tombstone.value : this.tombstone,
      updatedAt: data.updatedAt.present ? data.updatedAt.value : this.updatedAt,
    );
  }

  @override
  String toString() {
    return (StringBuffer('StoredRowState(')
          ..write('accountKey: $accountKey, ')
          ..write('tableId: $tableId, ')
          ..write('rowKey: $rowKey, ')
          ..write('phase: $phase, ')
          ..write('actionId: $actionId, ')
          ..write('attemptCount: $attemptCount, ')
          ..write('nextRetryAt: $nextRetryAt, ')
          ..write('errorCode: $errorCode, ')
          ..write('errorMessage: $errorMessage, ')
          ..write('lastServerSeq: $lastServerSeq, ')
          ..write('pendingValuesJson: $pendingValuesJson, ')
          ..write('tombstone: $tombstone, ')
          ..write('updatedAt: $updatedAt')
          ..write(')'))
        .toString();
  }

  @override
  int get hashCode => Object.hash(
    accountKey,
    tableId,
    rowKey,
    phase,
    actionId,
    attemptCount,
    nextRetryAt,
    errorCode,
    errorMessage,
    lastServerSeq,
    pendingValuesJson,
    tombstone,
    updatedAt,
  );
  @override
  bool operator ==(Object other) =>
      identical(this, other) ||
      (other is StoredRowState &&
          other.accountKey == this.accountKey &&
          other.tableId == this.tableId &&
          other.rowKey == this.rowKey &&
          other.phase == this.phase &&
          other.actionId == this.actionId &&
          other.attemptCount == this.attemptCount &&
          other.nextRetryAt == this.nextRetryAt &&
          other.errorCode == this.errorCode &&
          other.errorMessage == this.errorMessage &&
          other.lastServerSeq == this.lastServerSeq &&
          other.pendingValuesJson == this.pendingValuesJson &&
          other.tombstone == this.tombstone &&
          other.updatedAt == this.updatedAt);
}

class KalamRowStatesCompanion extends UpdateCompanion<StoredRowState> {
  final Value<String> accountKey;
  final Value<String> tableId;
  final Value<String> rowKey;
  final Value<String> phase;
  final Value<String?> actionId;
  final Value<int> attemptCount;
  final Value<DateTime?> nextRetryAt;
  final Value<String?> errorCode;
  final Value<String?> errorMessage;
  final Value<String?> lastServerSeq;
  final Value<String?> pendingValuesJson;
  final Value<bool> tombstone;
  final Value<DateTime> updatedAt;
  final Value<int> rowid;
  const KalamRowStatesCompanion({
    this.accountKey = const Value.absent(),
    this.tableId = const Value.absent(),
    this.rowKey = const Value.absent(),
    this.phase = const Value.absent(),
    this.actionId = const Value.absent(),
    this.attemptCount = const Value.absent(),
    this.nextRetryAt = const Value.absent(),
    this.errorCode = const Value.absent(),
    this.errorMessage = const Value.absent(),
    this.lastServerSeq = const Value.absent(),
    this.pendingValuesJson = const Value.absent(),
    this.tombstone = const Value.absent(),
    this.updatedAt = const Value.absent(),
    this.rowid = const Value.absent(),
  });
  KalamRowStatesCompanion.insert({
    required String accountKey,
    required String tableId,
    required String rowKey,
    required String phase,
    this.actionId = const Value.absent(),
    this.attemptCount = const Value.absent(),
    this.nextRetryAt = const Value.absent(),
    this.errorCode = const Value.absent(),
    this.errorMessage = const Value.absent(),
    this.lastServerSeq = const Value.absent(),
    this.pendingValuesJson = const Value.absent(),
    this.tombstone = const Value.absent(),
    required DateTime updatedAt,
    this.rowid = const Value.absent(),
  }) : accountKey = Value(accountKey),
       tableId = Value(tableId),
       rowKey = Value(rowKey),
       phase = Value(phase),
       updatedAt = Value(updatedAt);
  static Insertable<StoredRowState> custom({
    Expression<String>? accountKey,
    Expression<String>? tableId,
    Expression<String>? rowKey,
    Expression<String>? phase,
    Expression<String>? actionId,
    Expression<int>? attemptCount,
    Expression<DateTime>? nextRetryAt,
    Expression<String>? errorCode,
    Expression<String>? errorMessage,
    Expression<String>? lastServerSeq,
    Expression<String>? pendingValuesJson,
    Expression<bool>? tombstone,
    Expression<DateTime>? updatedAt,
    Expression<int>? rowid,
  }) {
    return RawValuesInsertable({
      if (accountKey != null) 'account_key': accountKey,
      if (tableId != null) 'table_id': tableId,
      if (rowKey != null) 'row_key': rowKey,
      if (phase != null) 'phase': phase,
      if (actionId != null) 'action_id': actionId,
      if (attemptCount != null) 'attempt_count': attemptCount,
      if (nextRetryAt != null) 'next_retry_at': nextRetryAt,
      if (errorCode != null) 'error_code': errorCode,
      if (errorMessage != null) 'error_message': errorMessage,
      if (lastServerSeq != null) 'last_server_seq': lastServerSeq,
      if (pendingValuesJson != null) 'pending_values_json': pendingValuesJson,
      if (tombstone != null) 'tombstone': tombstone,
      if (updatedAt != null) 'updated_at': updatedAt,
      if (rowid != null) 'rowid': rowid,
    });
  }

  KalamRowStatesCompanion copyWith({
    Value<String>? accountKey,
    Value<String>? tableId,
    Value<String>? rowKey,
    Value<String>? phase,
    Value<String?>? actionId,
    Value<int>? attemptCount,
    Value<DateTime?>? nextRetryAt,
    Value<String?>? errorCode,
    Value<String?>? errorMessage,
    Value<String?>? lastServerSeq,
    Value<String?>? pendingValuesJson,
    Value<bool>? tombstone,
    Value<DateTime>? updatedAt,
    Value<int>? rowid,
  }) {
    return KalamRowStatesCompanion(
      accountKey: accountKey ?? this.accountKey,
      tableId: tableId ?? this.tableId,
      rowKey: rowKey ?? this.rowKey,
      phase: phase ?? this.phase,
      actionId: actionId ?? this.actionId,
      attemptCount: attemptCount ?? this.attemptCount,
      nextRetryAt: nextRetryAt ?? this.nextRetryAt,
      errorCode: errorCode ?? this.errorCode,
      errorMessage: errorMessage ?? this.errorMessage,
      lastServerSeq: lastServerSeq ?? this.lastServerSeq,
      pendingValuesJson: pendingValuesJson ?? this.pendingValuesJson,
      tombstone: tombstone ?? this.tombstone,
      updatedAt: updatedAt ?? this.updatedAt,
      rowid: rowid ?? this.rowid,
    );
  }

  @override
  Map<String, Expression> toColumns(bool nullToAbsent) {
    final map = <String, Expression>{};
    if (accountKey.present) {
      map['account_key'] = Variable<String>(accountKey.value);
    }
    if (tableId.present) {
      map['table_id'] = Variable<String>(tableId.value);
    }
    if (rowKey.present) {
      map['row_key'] = Variable<String>(rowKey.value);
    }
    if (phase.present) {
      map['phase'] = Variable<String>(phase.value);
    }
    if (actionId.present) {
      map['action_id'] = Variable<String>(actionId.value);
    }
    if (attemptCount.present) {
      map['attempt_count'] = Variable<int>(attemptCount.value);
    }
    if (nextRetryAt.present) {
      map['next_retry_at'] = Variable<DateTime>(nextRetryAt.value);
    }
    if (errorCode.present) {
      map['error_code'] = Variable<String>(errorCode.value);
    }
    if (errorMessage.present) {
      map['error_message'] = Variable<String>(errorMessage.value);
    }
    if (lastServerSeq.present) {
      map['last_server_seq'] = Variable<String>(lastServerSeq.value);
    }
    if (pendingValuesJson.present) {
      map['pending_values_json'] = Variable<String>(pendingValuesJson.value);
    }
    if (tombstone.present) {
      map['tombstone'] = Variable<bool>(tombstone.value);
    }
    if (updatedAt.present) {
      map['updated_at'] = Variable<DateTime>(updatedAt.value);
    }
    if (rowid.present) {
      map['rowid'] = Variable<int>(rowid.value);
    }
    return map;
  }

  @override
  String toString() {
    return (StringBuffer('KalamRowStatesCompanion(')
          ..write('accountKey: $accountKey, ')
          ..write('tableId: $tableId, ')
          ..write('rowKey: $rowKey, ')
          ..write('phase: $phase, ')
          ..write('actionId: $actionId, ')
          ..write('attemptCount: $attemptCount, ')
          ..write('nextRetryAt: $nextRetryAt, ')
          ..write('errorCode: $errorCode, ')
          ..write('errorMessage: $errorMessage, ')
          ..write('lastServerSeq: $lastServerSeq, ')
          ..write('pendingValuesJson: $pendingValuesJson, ')
          ..write('tombstone: $tombstone, ')
          ..write('updatedAt: $updatedAt, ')
          ..write('rowid: $rowid')
          ..write(')'))
        .toString();
  }
}

abstract class _$KalamSyncDatabase extends GeneratedDatabase {
  _$KalamSyncDatabase(QueryExecutor e) : super(e);
  $KalamSyncDatabaseManager get managers => $KalamSyncDatabaseManager(this);
  late final $KalamActionsTable kalamActions = $KalamActionsTable(this);
  late final $KalamCachedRowsTable kalamCachedRows = $KalamCachedRowsTable(
    this,
  );
  late final $KalamActionStepsTable kalamActionSteps = $KalamActionStepsTable(
    this,
  );
  late final $KalamCheckpointsTable kalamCheckpoints = $KalamCheckpointsTable(
    this,
  );
  late final $KalamRowStatesTable kalamRowStates = $KalamRowStatesTable(this);
  @override
  Iterable<TableInfo<Table, Object?>> get allTables =>
      allSchemaEntities.whereType<TableInfo<Table, Object?>>();
  @override
  List<DatabaseSchemaEntity> get allSchemaEntities => [
    kalamActions,
    kalamCachedRows,
    kalamActionSteps,
    kalamCheckpoints,
    kalamRowStates,
  ];
}

typedef $$KalamActionsTableCreateCompanionBuilder =
    KalamActionsCompanion Function({
      required String id,
      required String accountKey,
      required String actionKey,
      Value<int> version,
      Value<String> kind,
      required String payloadJson,
      required String status,
      Value<String?> orderingKey,
      Value<String?> rowTableId,
      Value<String?> rowKey,
      Value<int> queuePosition,
      Value<int> attemptCount,
      Value<DateTime?> nextAttemptAt,
      Value<String?> lastError,
      required DateTime createdAt,
      required DateTime updatedAt,
      Value<int> rowid,
    });
typedef $$KalamActionsTableUpdateCompanionBuilder =
    KalamActionsCompanion Function({
      Value<String> id,
      Value<String> accountKey,
      Value<String> actionKey,
      Value<int> version,
      Value<String> kind,
      Value<String> payloadJson,
      Value<String> status,
      Value<String?> orderingKey,
      Value<String?> rowTableId,
      Value<String?> rowKey,
      Value<int> queuePosition,
      Value<int> attemptCount,
      Value<DateTime?> nextAttemptAt,
      Value<String?> lastError,
      Value<DateTime> createdAt,
      Value<DateTime> updatedAt,
      Value<int> rowid,
    });

class $$KalamActionsTableFilterComposer
    extends Composer<_$KalamSyncDatabase, $KalamActionsTable> {
  $$KalamActionsTableFilterComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  ColumnFilters<String> get id => $composableBuilder(
    column: $table.id,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get accountKey => $composableBuilder(
    column: $table.accountKey,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get actionKey => $composableBuilder(
    column: $table.actionKey,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<int> get version => $composableBuilder(
    column: $table.version,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get kind => $composableBuilder(
    column: $table.kind,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get payloadJson => $composableBuilder(
    column: $table.payloadJson,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get status => $composableBuilder(
    column: $table.status,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get orderingKey => $composableBuilder(
    column: $table.orderingKey,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get rowTableId => $composableBuilder(
    column: $table.rowTableId,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get rowKey => $composableBuilder(
    column: $table.rowKey,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<int> get queuePosition => $composableBuilder(
    column: $table.queuePosition,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<int> get attemptCount => $composableBuilder(
    column: $table.attemptCount,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<DateTime> get nextAttemptAt => $composableBuilder(
    column: $table.nextAttemptAt,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get lastError => $composableBuilder(
    column: $table.lastError,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<DateTime> get createdAt => $composableBuilder(
    column: $table.createdAt,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<DateTime> get updatedAt => $composableBuilder(
    column: $table.updatedAt,
    builder: (column) => ColumnFilters(column),
  );
}

class $$KalamActionsTableOrderingComposer
    extends Composer<_$KalamSyncDatabase, $KalamActionsTable> {
  $$KalamActionsTableOrderingComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  ColumnOrderings<String> get id => $composableBuilder(
    column: $table.id,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get accountKey => $composableBuilder(
    column: $table.accountKey,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get actionKey => $composableBuilder(
    column: $table.actionKey,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<int> get version => $composableBuilder(
    column: $table.version,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get kind => $composableBuilder(
    column: $table.kind,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get payloadJson => $composableBuilder(
    column: $table.payloadJson,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get status => $composableBuilder(
    column: $table.status,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get orderingKey => $composableBuilder(
    column: $table.orderingKey,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get rowTableId => $composableBuilder(
    column: $table.rowTableId,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get rowKey => $composableBuilder(
    column: $table.rowKey,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<int> get queuePosition => $composableBuilder(
    column: $table.queuePosition,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<int> get attemptCount => $composableBuilder(
    column: $table.attemptCount,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<DateTime> get nextAttemptAt => $composableBuilder(
    column: $table.nextAttemptAt,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get lastError => $composableBuilder(
    column: $table.lastError,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<DateTime> get createdAt => $composableBuilder(
    column: $table.createdAt,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<DateTime> get updatedAt => $composableBuilder(
    column: $table.updatedAt,
    builder: (column) => ColumnOrderings(column),
  );
}

class $$KalamActionsTableAnnotationComposer
    extends Composer<_$KalamSyncDatabase, $KalamActionsTable> {
  $$KalamActionsTableAnnotationComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  GeneratedColumn<String> get id =>
      $composableBuilder(column: $table.id, builder: (column) => column);

  GeneratedColumn<String> get accountKey => $composableBuilder(
    column: $table.accountKey,
    builder: (column) => column,
  );

  GeneratedColumn<String> get actionKey =>
      $composableBuilder(column: $table.actionKey, builder: (column) => column);

  GeneratedColumn<int> get version =>
      $composableBuilder(column: $table.version, builder: (column) => column);

  GeneratedColumn<String> get kind =>
      $composableBuilder(column: $table.kind, builder: (column) => column);

  GeneratedColumn<String> get payloadJson => $composableBuilder(
    column: $table.payloadJson,
    builder: (column) => column,
  );

  GeneratedColumn<String> get status =>
      $composableBuilder(column: $table.status, builder: (column) => column);

  GeneratedColumn<String> get orderingKey => $composableBuilder(
    column: $table.orderingKey,
    builder: (column) => column,
  );

  GeneratedColumn<String> get rowTableId => $composableBuilder(
    column: $table.rowTableId,
    builder: (column) => column,
  );

  GeneratedColumn<String> get rowKey =>
      $composableBuilder(column: $table.rowKey, builder: (column) => column);

  GeneratedColumn<int> get queuePosition => $composableBuilder(
    column: $table.queuePosition,
    builder: (column) => column,
  );

  GeneratedColumn<int> get attemptCount => $composableBuilder(
    column: $table.attemptCount,
    builder: (column) => column,
  );

  GeneratedColumn<DateTime> get nextAttemptAt => $composableBuilder(
    column: $table.nextAttemptAt,
    builder: (column) => column,
  );

  GeneratedColumn<String> get lastError =>
      $composableBuilder(column: $table.lastError, builder: (column) => column);

  GeneratedColumn<DateTime> get createdAt =>
      $composableBuilder(column: $table.createdAt, builder: (column) => column);

  GeneratedColumn<DateTime> get updatedAt =>
      $composableBuilder(column: $table.updatedAt, builder: (column) => column);
}

class $$KalamActionsTableTableManager
    extends
        RootTableManager<
          _$KalamSyncDatabase,
          $KalamActionsTable,
          StoredAction,
          $$KalamActionsTableFilterComposer,
          $$KalamActionsTableOrderingComposer,
          $$KalamActionsTableAnnotationComposer,
          $$KalamActionsTableCreateCompanionBuilder,
          $$KalamActionsTableUpdateCompanionBuilder,
          (
            StoredAction,
            BaseReferences<
              _$KalamSyncDatabase,
              $KalamActionsTable,
              StoredAction
            >,
          ),
          StoredAction,
          PrefetchHooks Function()
        > {
  $$KalamActionsTableTableManager(
    _$KalamSyncDatabase db,
    $KalamActionsTable table,
  ) : super(
        TableManagerState(
          db: db,
          table: table,
          createFilteringComposer: () =>
              $$KalamActionsTableFilterComposer($db: db, $table: table),
          createOrderingComposer: () =>
              $$KalamActionsTableOrderingComposer($db: db, $table: table),
          createComputedFieldComposer: () =>
              $$KalamActionsTableAnnotationComposer($db: db, $table: table),
          updateCompanionCallback:
              ({
                Value<String> id = const Value.absent(),
                Value<String> accountKey = const Value.absent(),
                Value<String> actionKey = const Value.absent(),
                Value<int> version = const Value.absent(),
                Value<String> kind = const Value.absent(),
                Value<String> payloadJson = const Value.absent(),
                Value<String> status = const Value.absent(),
                Value<String?> orderingKey = const Value.absent(),
                Value<String?> rowTableId = const Value.absent(),
                Value<String?> rowKey = const Value.absent(),
                Value<int> queuePosition = const Value.absent(),
                Value<int> attemptCount = const Value.absent(),
                Value<DateTime?> nextAttemptAt = const Value.absent(),
                Value<String?> lastError = const Value.absent(),
                Value<DateTime> createdAt = const Value.absent(),
                Value<DateTime> updatedAt = const Value.absent(),
                Value<int> rowid = const Value.absent(),
              }) => KalamActionsCompanion(
                id: id,
                accountKey: accountKey,
                actionKey: actionKey,
                version: version,
                kind: kind,
                payloadJson: payloadJson,
                status: status,
                orderingKey: orderingKey,
                rowTableId: rowTableId,
                rowKey: rowKey,
                queuePosition: queuePosition,
                attemptCount: attemptCount,
                nextAttemptAt: nextAttemptAt,
                lastError: lastError,
                createdAt: createdAt,
                updatedAt: updatedAt,
                rowid: rowid,
              ),
          createCompanionCallback:
              ({
                required String id,
                required String accountKey,
                required String actionKey,
                Value<int> version = const Value.absent(),
                Value<String> kind = const Value.absent(),
                required String payloadJson,
                required String status,
                Value<String?> orderingKey = const Value.absent(),
                Value<String?> rowTableId = const Value.absent(),
                Value<String?> rowKey = const Value.absent(),
                Value<int> queuePosition = const Value.absent(),
                Value<int> attemptCount = const Value.absent(),
                Value<DateTime?> nextAttemptAt = const Value.absent(),
                Value<String?> lastError = const Value.absent(),
                required DateTime createdAt,
                required DateTime updatedAt,
                Value<int> rowid = const Value.absent(),
              }) => KalamActionsCompanion.insert(
                id: id,
                accountKey: accountKey,
                actionKey: actionKey,
                version: version,
                kind: kind,
                payloadJson: payloadJson,
                status: status,
                orderingKey: orderingKey,
                rowTableId: rowTableId,
                rowKey: rowKey,
                queuePosition: queuePosition,
                attemptCount: attemptCount,
                nextAttemptAt: nextAttemptAt,
                lastError: lastError,
                createdAt: createdAt,
                updatedAt: updatedAt,
                rowid: rowid,
              ),
          withReferenceMapper: (p0) => p0
              .map((e) => (e.readTable(table), BaseReferences(db, table, e)))
              .toList(),
          prefetchHooksCallback: null,
        ),
      );
}

typedef $$KalamActionsTableProcessedTableManager =
    ProcessedTableManager<
      _$KalamSyncDatabase,
      $KalamActionsTable,
      StoredAction,
      $$KalamActionsTableFilterComposer,
      $$KalamActionsTableOrderingComposer,
      $$KalamActionsTableAnnotationComposer,
      $$KalamActionsTableCreateCompanionBuilder,
      $$KalamActionsTableUpdateCompanionBuilder,
      (
        StoredAction,
        BaseReferences<_$KalamSyncDatabase, $KalamActionsTable, StoredAction>,
      ),
      StoredAction,
      PrefetchHooks Function()
    >;
typedef $$KalamCachedRowsTableCreateCompanionBuilder =
    KalamCachedRowsCompanion Function({
      required String accountKey,
      required String tableId,
      required String rowKey,
      required String valuesJson,
      required DateTime updatedAt,
      Value<int> rowid,
    });
typedef $$KalamCachedRowsTableUpdateCompanionBuilder =
    KalamCachedRowsCompanion Function({
      Value<String> accountKey,
      Value<String> tableId,
      Value<String> rowKey,
      Value<String> valuesJson,
      Value<DateTime> updatedAt,
      Value<int> rowid,
    });

class $$KalamCachedRowsTableFilterComposer
    extends Composer<_$KalamSyncDatabase, $KalamCachedRowsTable> {
  $$KalamCachedRowsTableFilterComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  ColumnFilters<String> get accountKey => $composableBuilder(
    column: $table.accountKey,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get tableId => $composableBuilder(
    column: $table.tableId,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get rowKey => $composableBuilder(
    column: $table.rowKey,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get valuesJson => $composableBuilder(
    column: $table.valuesJson,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<DateTime> get updatedAt => $composableBuilder(
    column: $table.updatedAt,
    builder: (column) => ColumnFilters(column),
  );
}

class $$KalamCachedRowsTableOrderingComposer
    extends Composer<_$KalamSyncDatabase, $KalamCachedRowsTable> {
  $$KalamCachedRowsTableOrderingComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  ColumnOrderings<String> get accountKey => $composableBuilder(
    column: $table.accountKey,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get tableId => $composableBuilder(
    column: $table.tableId,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get rowKey => $composableBuilder(
    column: $table.rowKey,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get valuesJson => $composableBuilder(
    column: $table.valuesJson,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<DateTime> get updatedAt => $composableBuilder(
    column: $table.updatedAt,
    builder: (column) => ColumnOrderings(column),
  );
}

class $$KalamCachedRowsTableAnnotationComposer
    extends Composer<_$KalamSyncDatabase, $KalamCachedRowsTable> {
  $$KalamCachedRowsTableAnnotationComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  GeneratedColumn<String> get accountKey => $composableBuilder(
    column: $table.accountKey,
    builder: (column) => column,
  );

  GeneratedColumn<String> get tableId =>
      $composableBuilder(column: $table.tableId, builder: (column) => column);

  GeneratedColumn<String> get rowKey =>
      $composableBuilder(column: $table.rowKey, builder: (column) => column);

  GeneratedColumn<String> get valuesJson => $composableBuilder(
    column: $table.valuesJson,
    builder: (column) => column,
  );

  GeneratedColumn<DateTime> get updatedAt =>
      $composableBuilder(column: $table.updatedAt, builder: (column) => column);
}

class $$KalamCachedRowsTableTableManager
    extends
        RootTableManager<
          _$KalamSyncDatabase,
          $KalamCachedRowsTable,
          StoredCachedRow,
          $$KalamCachedRowsTableFilterComposer,
          $$KalamCachedRowsTableOrderingComposer,
          $$KalamCachedRowsTableAnnotationComposer,
          $$KalamCachedRowsTableCreateCompanionBuilder,
          $$KalamCachedRowsTableUpdateCompanionBuilder,
          (
            StoredCachedRow,
            BaseReferences<
              _$KalamSyncDatabase,
              $KalamCachedRowsTable,
              StoredCachedRow
            >,
          ),
          StoredCachedRow,
          PrefetchHooks Function()
        > {
  $$KalamCachedRowsTableTableManager(
    _$KalamSyncDatabase db,
    $KalamCachedRowsTable table,
  ) : super(
        TableManagerState(
          db: db,
          table: table,
          createFilteringComposer: () =>
              $$KalamCachedRowsTableFilterComposer($db: db, $table: table),
          createOrderingComposer: () =>
              $$KalamCachedRowsTableOrderingComposer($db: db, $table: table),
          createComputedFieldComposer: () =>
              $$KalamCachedRowsTableAnnotationComposer($db: db, $table: table),
          updateCompanionCallback:
              ({
                Value<String> accountKey = const Value.absent(),
                Value<String> tableId = const Value.absent(),
                Value<String> rowKey = const Value.absent(),
                Value<String> valuesJson = const Value.absent(),
                Value<DateTime> updatedAt = const Value.absent(),
                Value<int> rowid = const Value.absent(),
              }) => KalamCachedRowsCompanion(
                accountKey: accountKey,
                tableId: tableId,
                rowKey: rowKey,
                valuesJson: valuesJson,
                updatedAt: updatedAt,
                rowid: rowid,
              ),
          createCompanionCallback:
              ({
                required String accountKey,
                required String tableId,
                required String rowKey,
                required String valuesJson,
                required DateTime updatedAt,
                Value<int> rowid = const Value.absent(),
              }) => KalamCachedRowsCompanion.insert(
                accountKey: accountKey,
                tableId: tableId,
                rowKey: rowKey,
                valuesJson: valuesJson,
                updatedAt: updatedAt,
                rowid: rowid,
              ),
          withReferenceMapper: (p0) => p0
              .map((e) => (e.readTable(table), BaseReferences(db, table, e)))
              .toList(),
          prefetchHooksCallback: null,
        ),
      );
}

typedef $$KalamCachedRowsTableProcessedTableManager =
    ProcessedTableManager<
      _$KalamSyncDatabase,
      $KalamCachedRowsTable,
      StoredCachedRow,
      $$KalamCachedRowsTableFilterComposer,
      $$KalamCachedRowsTableOrderingComposer,
      $$KalamCachedRowsTableAnnotationComposer,
      $$KalamCachedRowsTableCreateCompanionBuilder,
      $$KalamCachedRowsTableUpdateCompanionBuilder,
      (
        StoredCachedRow,
        BaseReferences<
          _$KalamSyncDatabase,
          $KalamCachedRowsTable,
          StoredCachedRow
        >,
      ),
      StoredCachedRow,
      PrefetchHooks Function()
    >;
typedef $$KalamActionStepsTableCreateCompanionBuilder =
    KalamActionStepsCompanion Function({
      required String actionId,
      required String name,
      required String status,
      Value<String?> resultJson,
      Value<String?> lastError,
      required DateTime updatedAt,
      Value<int> rowid,
    });
typedef $$KalamActionStepsTableUpdateCompanionBuilder =
    KalamActionStepsCompanion Function({
      Value<String> actionId,
      Value<String> name,
      Value<String> status,
      Value<String?> resultJson,
      Value<String?> lastError,
      Value<DateTime> updatedAt,
      Value<int> rowid,
    });

class $$KalamActionStepsTableFilterComposer
    extends Composer<_$KalamSyncDatabase, $KalamActionStepsTable> {
  $$KalamActionStepsTableFilterComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  ColumnFilters<String> get actionId => $composableBuilder(
    column: $table.actionId,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get name => $composableBuilder(
    column: $table.name,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get status => $composableBuilder(
    column: $table.status,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get resultJson => $composableBuilder(
    column: $table.resultJson,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get lastError => $composableBuilder(
    column: $table.lastError,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<DateTime> get updatedAt => $composableBuilder(
    column: $table.updatedAt,
    builder: (column) => ColumnFilters(column),
  );
}

class $$KalamActionStepsTableOrderingComposer
    extends Composer<_$KalamSyncDatabase, $KalamActionStepsTable> {
  $$KalamActionStepsTableOrderingComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  ColumnOrderings<String> get actionId => $composableBuilder(
    column: $table.actionId,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get name => $composableBuilder(
    column: $table.name,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get status => $composableBuilder(
    column: $table.status,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get resultJson => $composableBuilder(
    column: $table.resultJson,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get lastError => $composableBuilder(
    column: $table.lastError,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<DateTime> get updatedAt => $composableBuilder(
    column: $table.updatedAt,
    builder: (column) => ColumnOrderings(column),
  );
}

class $$KalamActionStepsTableAnnotationComposer
    extends Composer<_$KalamSyncDatabase, $KalamActionStepsTable> {
  $$KalamActionStepsTableAnnotationComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  GeneratedColumn<String> get actionId =>
      $composableBuilder(column: $table.actionId, builder: (column) => column);

  GeneratedColumn<String> get name =>
      $composableBuilder(column: $table.name, builder: (column) => column);

  GeneratedColumn<String> get status =>
      $composableBuilder(column: $table.status, builder: (column) => column);

  GeneratedColumn<String> get resultJson => $composableBuilder(
    column: $table.resultJson,
    builder: (column) => column,
  );

  GeneratedColumn<String> get lastError =>
      $composableBuilder(column: $table.lastError, builder: (column) => column);

  GeneratedColumn<DateTime> get updatedAt =>
      $composableBuilder(column: $table.updatedAt, builder: (column) => column);
}

class $$KalamActionStepsTableTableManager
    extends
        RootTableManager<
          _$KalamSyncDatabase,
          $KalamActionStepsTable,
          StoredActionStep,
          $$KalamActionStepsTableFilterComposer,
          $$KalamActionStepsTableOrderingComposer,
          $$KalamActionStepsTableAnnotationComposer,
          $$KalamActionStepsTableCreateCompanionBuilder,
          $$KalamActionStepsTableUpdateCompanionBuilder,
          (
            StoredActionStep,
            BaseReferences<
              _$KalamSyncDatabase,
              $KalamActionStepsTable,
              StoredActionStep
            >,
          ),
          StoredActionStep,
          PrefetchHooks Function()
        > {
  $$KalamActionStepsTableTableManager(
    _$KalamSyncDatabase db,
    $KalamActionStepsTable table,
  ) : super(
        TableManagerState(
          db: db,
          table: table,
          createFilteringComposer: () =>
              $$KalamActionStepsTableFilterComposer($db: db, $table: table),
          createOrderingComposer: () =>
              $$KalamActionStepsTableOrderingComposer($db: db, $table: table),
          createComputedFieldComposer: () =>
              $$KalamActionStepsTableAnnotationComposer($db: db, $table: table),
          updateCompanionCallback:
              ({
                Value<String> actionId = const Value.absent(),
                Value<String> name = const Value.absent(),
                Value<String> status = const Value.absent(),
                Value<String?> resultJson = const Value.absent(),
                Value<String?> lastError = const Value.absent(),
                Value<DateTime> updatedAt = const Value.absent(),
                Value<int> rowid = const Value.absent(),
              }) => KalamActionStepsCompanion(
                actionId: actionId,
                name: name,
                status: status,
                resultJson: resultJson,
                lastError: lastError,
                updatedAt: updatedAt,
                rowid: rowid,
              ),
          createCompanionCallback:
              ({
                required String actionId,
                required String name,
                required String status,
                Value<String?> resultJson = const Value.absent(),
                Value<String?> lastError = const Value.absent(),
                required DateTime updatedAt,
                Value<int> rowid = const Value.absent(),
              }) => KalamActionStepsCompanion.insert(
                actionId: actionId,
                name: name,
                status: status,
                resultJson: resultJson,
                lastError: lastError,
                updatedAt: updatedAt,
                rowid: rowid,
              ),
          withReferenceMapper: (p0) => p0
              .map((e) => (e.readTable(table), BaseReferences(db, table, e)))
              .toList(),
          prefetchHooksCallback: null,
        ),
      );
}

typedef $$KalamActionStepsTableProcessedTableManager =
    ProcessedTableManager<
      _$KalamSyncDatabase,
      $KalamActionStepsTable,
      StoredActionStep,
      $$KalamActionStepsTableFilterComposer,
      $$KalamActionStepsTableOrderingComposer,
      $$KalamActionStepsTableAnnotationComposer,
      $$KalamActionStepsTableCreateCompanionBuilder,
      $$KalamActionStepsTableUpdateCompanionBuilder,
      (
        StoredActionStep,
        BaseReferences<
          _$KalamSyncDatabase,
          $KalamActionStepsTable,
          StoredActionStep
        >,
      ),
      StoredActionStep,
      PrefetchHooks Function()
    >;
typedef $$KalamCheckpointsTableCreateCompanionBuilder =
    KalamCheckpointsCompanion Function({
      required String accountKey,
      required String subscriptionId,
      required String seq,
      required DateTime updatedAt,
      Value<int> rowid,
    });
typedef $$KalamCheckpointsTableUpdateCompanionBuilder =
    KalamCheckpointsCompanion Function({
      Value<String> accountKey,
      Value<String> subscriptionId,
      Value<String> seq,
      Value<DateTime> updatedAt,
      Value<int> rowid,
    });

class $$KalamCheckpointsTableFilterComposer
    extends Composer<_$KalamSyncDatabase, $KalamCheckpointsTable> {
  $$KalamCheckpointsTableFilterComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  ColumnFilters<String> get accountKey => $composableBuilder(
    column: $table.accountKey,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get subscriptionId => $composableBuilder(
    column: $table.subscriptionId,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get seq => $composableBuilder(
    column: $table.seq,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<DateTime> get updatedAt => $composableBuilder(
    column: $table.updatedAt,
    builder: (column) => ColumnFilters(column),
  );
}

class $$KalamCheckpointsTableOrderingComposer
    extends Composer<_$KalamSyncDatabase, $KalamCheckpointsTable> {
  $$KalamCheckpointsTableOrderingComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  ColumnOrderings<String> get accountKey => $composableBuilder(
    column: $table.accountKey,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get subscriptionId => $composableBuilder(
    column: $table.subscriptionId,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get seq => $composableBuilder(
    column: $table.seq,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<DateTime> get updatedAt => $composableBuilder(
    column: $table.updatedAt,
    builder: (column) => ColumnOrderings(column),
  );
}

class $$KalamCheckpointsTableAnnotationComposer
    extends Composer<_$KalamSyncDatabase, $KalamCheckpointsTable> {
  $$KalamCheckpointsTableAnnotationComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  GeneratedColumn<String> get accountKey => $composableBuilder(
    column: $table.accountKey,
    builder: (column) => column,
  );

  GeneratedColumn<String> get subscriptionId => $composableBuilder(
    column: $table.subscriptionId,
    builder: (column) => column,
  );

  GeneratedColumn<String> get seq =>
      $composableBuilder(column: $table.seq, builder: (column) => column);

  GeneratedColumn<DateTime> get updatedAt =>
      $composableBuilder(column: $table.updatedAt, builder: (column) => column);
}

class $$KalamCheckpointsTableTableManager
    extends
        RootTableManager<
          _$KalamSyncDatabase,
          $KalamCheckpointsTable,
          StoredCheckpoint,
          $$KalamCheckpointsTableFilterComposer,
          $$KalamCheckpointsTableOrderingComposer,
          $$KalamCheckpointsTableAnnotationComposer,
          $$KalamCheckpointsTableCreateCompanionBuilder,
          $$KalamCheckpointsTableUpdateCompanionBuilder,
          (
            StoredCheckpoint,
            BaseReferences<
              _$KalamSyncDatabase,
              $KalamCheckpointsTable,
              StoredCheckpoint
            >,
          ),
          StoredCheckpoint,
          PrefetchHooks Function()
        > {
  $$KalamCheckpointsTableTableManager(
    _$KalamSyncDatabase db,
    $KalamCheckpointsTable table,
  ) : super(
        TableManagerState(
          db: db,
          table: table,
          createFilteringComposer: () =>
              $$KalamCheckpointsTableFilterComposer($db: db, $table: table),
          createOrderingComposer: () =>
              $$KalamCheckpointsTableOrderingComposer($db: db, $table: table),
          createComputedFieldComposer: () =>
              $$KalamCheckpointsTableAnnotationComposer($db: db, $table: table),
          updateCompanionCallback:
              ({
                Value<String> accountKey = const Value.absent(),
                Value<String> subscriptionId = const Value.absent(),
                Value<String> seq = const Value.absent(),
                Value<DateTime> updatedAt = const Value.absent(),
                Value<int> rowid = const Value.absent(),
              }) => KalamCheckpointsCompanion(
                accountKey: accountKey,
                subscriptionId: subscriptionId,
                seq: seq,
                updatedAt: updatedAt,
                rowid: rowid,
              ),
          createCompanionCallback:
              ({
                required String accountKey,
                required String subscriptionId,
                required String seq,
                required DateTime updatedAt,
                Value<int> rowid = const Value.absent(),
              }) => KalamCheckpointsCompanion.insert(
                accountKey: accountKey,
                subscriptionId: subscriptionId,
                seq: seq,
                updatedAt: updatedAt,
                rowid: rowid,
              ),
          withReferenceMapper: (p0) => p0
              .map((e) => (e.readTable(table), BaseReferences(db, table, e)))
              .toList(),
          prefetchHooksCallback: null,
        ),
      );
}

typedef $$KalamCheckpointsTableProcessedTableManager =
    ProcessedTableManager<
      _$KalamSyncDatabase,
      $KalamCheckpointsTable,
      StoredCheckpoint,
      $$KalamCheckpointsTableFilterComposer,
      $$KalamCheckpointsTableOrderingComposer,
      $$KalamCheckpointsTableAnnotationComposer,
      $$KalamCheckpointsTableCreateCompanionBuilder,
      $$KalamCheckpointsTableUpdateCompanionBuilder,
      (
        StoredCheckpoint,
        BaseReferences<
          _$KalamSyncDatabase,
          $KalamCheckpointsTable,
          StoredCheckpoint
        >,
      ),
      StoredCheckpoint,
      PrefetchHooks Function()
    >;
typedef $$KalamRowStatesTableCreateCompanionBuilder =
    KalamRowStatesCompanion Function({
      required String accountKey,
      required String tableId,
      required String rowKey,
      required String phase,
      Value<String?> actionId,
      Value<int> attemptCount,
      Value<DateTime?> nextRetryAt,
      Value<String?> errorCode,
      Value<String?> errorMessage,
      Value<String?> lastServerSeq,
      Value<String?> pendingValuesJson,
      Value<bool> tombstone,
      required DateTime updatedAt,
      Value<int> rowid,
    });
typedef $$KalamRowStatesTableUpdateCompanionBuilder =
    KalamRowStatesCompanion Function({
      Value<String> accountKey,
      Value<String> tableId,
      Value<String> rowKey,
      Value<String> phase,
      Value<String?> actionId,
      Value<int> attemptCount,
      Value<DateTime?> nextRetryAt,
      Value<String?> errorCode,
      Value<String?> errorMessage,
      Value<String?> lastServerSeq,
      Value<String?> pendingValuesJson,
      Value<bool> tombstone,
      Value<DateTime> updatedAt,
      Value<int> rowid,
    });

class $$KalamRowStatesTableFilterComposer
    extends Composer<_$KalamSyncDatabase, $KalamRowStatesTable> {
  $$KalamRowStatesTableFilterComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  ColumnFilters<String> get accountKey => $composableBuilder(
    column: $table.accountKey,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get tableId => $composableBuilder(
    column: $table.tableId,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get rowKey => $composableBuilder(
    column: $table.rowKey,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get phase => $composableBuilder(
    column: $table.phase,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get actionId => $composableBuilder(
    column: $table.actionId,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<int> get attemptCount => $composableBuilder(
    column: $table.attemptCount,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<DateTime> get nextRetryAt => $composableBuilder(
    column: $table.nextRetryAt,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get errorCode => $composableBuilder(
    column: $table.errorCode,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get errorMessage => $composableBuilder(
    column: $table.errorMessage,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get lastServerSeq => $composableBuilder(
    column: $table.lastServerSeq,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<String> get pendingValuesJson => $composableBuilder(
    column: $table.pendingValuesJson,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<bool> get tombstone => $composableBuilder(
    column: $table.tombstone,
    builder: (column) => ColumnFilters(column),
  );

  ColumnFilters<DateTime> get updatedAt => $composableBuilder(
    column: $table.updatedAt,
    builder: (column) => ColumnFilters(column),
  );
}

class $$KalamRowStatesTableOrderingComposer
    extends Composer<_$KalamSyncDatabase, $KalamRowStatesTable> {
  $$KalamRowStatesTableOrderingComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  ColumnOrderings<String> get accountKey => $composableBuilder(
    column: $table.accountKey,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get tableId => $composableBuilder(
    column: $table.tableId,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get rowKey => $composableBuilder(
    column: $table.rowKey,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get phase => $composableBuilder(
    column: $table.phase,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get actionId => $composableBuilder(
    column: $table.actionId,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<int> get attemptCount => $composableBuilder(
    column: $table.attemptCount,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<DateTime> get nextRetryAt => $composableBuilder(
    column: $table.nextRetryAt,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get errorCode => $composableBuilder(
    column: $table.errorCode,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get errorMessage => $composableBuilder(
    column: $table.errorMessage,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get lastServerSeq => $composableBuilder(
    column: $table.lastServerSeq,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<String> get pendingValuesJson => $composableBuilder(
    column: $table.pendingValuesJson,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<bool> get tombstone => $composableBuilder(
    column: $table.tombstone,
    builder: (column) => ColumnOrderings(column),
  );

  ColumnOrderings<DateTime> get updatedAt => $composableBuilder(
    column: $table.updatedAt,
    builder: (column) => ColumnOrderings(column),
  );
}

class $$KalamRowStatesTableAnnotationComposer
    extends Composer<_$KalamSyncDatabase, $KalamRowStatesTable> {
  $$KalamRowStatesTableAnnotationComposer({
    required super.$db,
    required super.$table,
    super.joinBuilder,
    super.$addJoinBuilderToRootComposer,
    super.$removeJoinBuilderFromRootComposer,
  });
  GeneratedColumn<String> get accountKey => $composableBuilder(
    column: $table.accountKey,
    builder: (column) => column,
  );

  GeneratedColumn<String> get tableId =>
      $composableBuilder(column: $table.tableId, builder: (column) => column);

  GeneratedColumn<String> get rowKey =>
      $composableBuilder(column: $table.rowKey, builder: (column) => column);

  GeneratedColumn<String> get phase =>
      $composableBuilder(column: $table.phase, builder: (column) => column);

  GeneratedColumn<String> get actionId =>
      $composableBuilder(column: $table.actionId, builder: (column) => column);

  GeneratedColumn<int> get attemptCount => $composableBuilder(
    column: $table.attemptCount,
    builder: (column) => column,
  );

  GeneratedColumn<DateTime> get nextRetryAt => $composableBuilder(
    column: $table.nextRetryAt,
    builder: (column) => column,
  );

  GeneratedColumn<String> get errorCode =>
      $composableBuilder(column: $table.errorCode, builder: (column) => column);

  GeneratedColumn<String> get errorMessage => $composableBuilder(
    column: $table.errorMessage,
    builder: (column) => column,
  );

  GeneratedColumn<String> get lastServerSeq => $composableBuilder(
    column: $table.lastServerSeq,
    builder: (column) => column,
  );

  GeneratedColumn<String> get pendingValuesJson => $composableBuilder(
    column: $table.pendingValuesJson,
    builder: (column) => column,
  );

  GeneratedColumn<bool> get tombstone =>
      $composableBuilder(column: $table.tombstone, builder: (column) => column);

  GeneratedColumn<DateTime> get updatedAt =>
      $composableBuilder(column: $table.updatedAt, builder: (column) => column);
}

class $$KalamRowStatesTableTableManager
    extends
        RootTableManager<
          _$KalamSyncDatabase,
          $KalamRowStatesTable,
          StoredRowState,
          $$KalamRowStatesTableFilterComposer,
          $$KalamRowStatesTableOrderingComposer,
          $$KalamRowStatesTableAnnotationComposer,
          $$KalamRowStatesTableCreateCompanionBuilder,
          $$KalamRowStatesTableUpdateCompanionBuilder,
          (
            StoredRowState,
            BaseReferences<
              _$KalamSyncDatabase,
              $KalamRowStatesTable,
              StoredRowState
            >,
          ),
          StoredRowState,
          PrefetchHooks Function()
        > {
  $$KalamRowStatesTableTableManager(
    _$KalamSyncDatabase db,
    $KalamRowStatesTable table,
  ) : super(
        TableManagerState(
          db: db,
          table: table,
          createFilteringComposer: () =>
              $$KalamRowStatesTableFilterComposer($db: db, $table: table),
          createOrderingComposer: () =>
              $$KalamRowStatesTableOrderingComposer($db: db, $table: table),
          createComputedFieldComposer: () =>
              $$KalamRowStatesTableAnnotationComposer($db: db, $table: table),
          updateCompanionCallback:
              ({
                Value<String> accountKey = const Value.absent(),
                Value<String> tableId = const Value.absent(),
                Value<String> rowKey = const Value.absent(),
                Value<String> phase = const Value.absent(),
                Value<String?> actionId = const Value.absent(),
                Value<int> attemptCount = const Value.absent(),
                Value<DateTime?> nextRetryAt = const Value.absent(),
                Value<String?> errorCode = const Value.absent(),
                Value<String?> errorMessage = const Value.absent(),
                Value<String?> lastServerSeq = const Value.absent(),
                Value<String?> pendingValuesJson = const Value.absent(),
                Value<bool> tombstone = const Value.absent(),
                Value<DateTime> updatedAt = const Value.absent(),
                Value<int> rowid = const Value.absent(),
              }) => KalamRowStatesCompanion(
                accountKey: accountKey,
                tableId: tableId,
                rowKey: rowKey,
                phase: phase,
                actionId: actionId,
                attemptCount: attemptCount,
                nextRetryAt: nextRetryAt,
                errorCode: errorCode,
                errorMessage: errorMessage,
                lastServerSeq: lastServerSeq,
                pendingValuesJson: pendingValuesJson,
                tombstone: tombstone,
                updatedAt: updatedAt,
                rowid: rowid,
              ),
          createCompanionCallback:
              ({
                required String accountKey,
                required String tableId,
                required String rowKey,
                required String phase,
                Value<String?> actionId = const Value.absent(),
                Value<int> attemptCount = const Value.absent(),
                Value<DateTime?> nextRetryAt = const Value.absent(),
                Value<String?> errorCode = const Value.absent(),
                Value<String?> errorMessage = const Value.absent(),
                Value<String?> lastServerSeq = const Value.absent(),
                Value<String?> pendingValuesJson = const Value.absent(),
                Value<bool> tombstone = const Value.absent(),
                required DateTime updatedAt,
                Value<int> rowid = const Value.absent(),
              }) => KalamRowStatesCompanion.insert(
                accountKey: accountKey,
                tableId: tableId,
                rowKey: rowKey,
                phase: phase,
                actionId: actionId,
                attemptCount: attemptCount,
                nextRetryAt: nextRetryAt,
                errorCode: errorCode,
                errorMessage: errorMessage,
                lastServerSeq: lastServerSeq,
                pendingValuesJson: pendingValuesJson,
                tombstone: tombstone,
                updatedAt: updatedAt,
                rowid: rowid,
              ),
          withReferenceMapper: (p0) => p0
              .map((e) => (e.readTable(table), BaseReferences(db, table, e)))
              .toList(),
          prefetchHooksCallback: null,
        ),
      );
}

typedef $$KalamRowStatesTableProcessedTableManager =
    ProcessedTableManager<
      _$KalamSyncDatabase,
      $KalamRowStatesTable,
      StoredRowState,
      $$KalamRowStatesTableFilterComposer,
      $$KalamRowStatesTableOrderingComposer,
      $$KalamRowStatesTableAnnotationComposer,
      $$KalamRowStatesTableCreateCompanionBuilder,
      $$KalamRowStatesTableUpdateCompanionBuilder,
      (
        StoredRowState,
        BaseReferences<
          _$KalamSyncDatabase,
          $KalamRowStatesTable,
          StoredRowState
        >,
      ),
      StoredRowState,
      PrefetchHooks Function()
    >;

class $KalamSyncDatabaseManager {
  final _$KalamSyncDatabase _db;
  $KalamSyncDatabaseManager(this._db);
  $$KalamActionsTableTableManager get kalamActions =>
      $$KalamActionsTableTableManager(_db, _db.kalamActions);
  $$KalamCachedRowsTableTableManager get kalamCachedRows =>
      $$KalamCachedRowsTableTableManager(_db, _db.kalamCachedRows);
  $$KalamActionStepsTableTableManager get kalamActionSteps =>
      $$KalamActionStepsTableTableManager(_db, _db.kalamActionSteps);
  $$KalamCheckpointsTableTableManager get kalamCheckpoints =>
      $$KalamCheckpointsTableTableManager(_db, _db.kalamCheckpoints);
  $$KalamRowStatesTableTableManager get kalamRowStates =>
      $$KalamRowStatesTableTableManager(_db, _db.kalamRowStates);
}
