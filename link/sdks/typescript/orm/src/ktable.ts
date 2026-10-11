import type { Table } from 'drizzle-orm';
import type { BuildColumns, BuildExtraConfigColumns } from 'drizzle-orm/column-builder';
import type { PgColumnBuilderBase } from 'drizzle-orm/pg-core';
import type { PgTableExtraConfig, PgTableExtraConfigValue, PgTableWithColumns } from 'drizzle-orm/pg-core/table';
import { boolean, pgSchema, pgTable, text } from 'drizzle-orm/pg-core';

export type KalamTableType = 'shared' | 'user' | 'stream' | 'system';

export type KalamSystemColumnName = '_version' | '_deleted' | '_timestamp';

export interface KalamTableOptions {
	/**
	 * KalamDB table kind. This is stored as metadata for agent, consumer, and live-query helpers that
	 * need to distinguish shared user-visible data from per-user, stream, or system tables.
	 */
	tableType?: KalamTableType;

	/**
	 * Adds KalamDB-owned columns to the Drizzle table shape. `true` adds `_version` and `_deleted`
	 * for shared and user tables, and `_version` for streams. `all` also adds `_timestamp`.
	 */
	systemColumns?: boolean | 'all' | readonly KalamSystemColumnName[];
}

export interface KalamOrmConfig {
	namespace?: string;
}

export interface KalamTableConfig {
	qualifiedName: string;
	namespace?: string;
	name: string;
	tableId: string;
	tableType?: KalamTableType;
	systemColumns: readonly KalamSystemColumnName[];
}

type ColumnMap = Record<string, PgColumnBuilderBase>;
type ColumnFactory = (columnTypes: unknown) => ColumnMap;
type ColumnsInput = ColumnMap | ColumnFactory;
type ExtraConfig = (self: unknown) => unknown;
type KalamPgTableWithColumns<TTableName extends string, TColumnsMap extends ColumnMap> = PgTableWithColumns<{
	name: TTableName;
	schema: undefined;
	columns: BuildColumns<TTableName, TColumnsMap, 'pg'>;
	dialect: 'pg';
}>;
type ExtraConfigArray<TTableName extends string, TColumnsMap extends ColumnMap> = (
	self: BuildExtraConfigColumns<TTableName, TColumnsMap, 'pg'>,
) => PgTableExtraConfigValue[];
type ExtraConfigObject<TTableName extends string, TColumnsMap extends ColumnMap> = (
	self: BuildExtraConfigColumns<TTableName, TColumnsMap, 'pg'>,
) => PgTableExtraConfig;
type VersionSystemColumnBuilders = { _version: ReturnType<typeof text> };
type DeletedSystemColumnBuilders = { _deleted: ReturnType<typeof boolean> };
type TimestampSystemColumnBuilders = { _timestamp: ReturnType<typeof text> };
type PublicVersionSystemColumnBuilders = VersionSystemColumnBuilders & DeletedSystemColumnBuilders;
type AllSystemColumnBuilders = PublicVersionSystemColumnBuilders & TimestampSystemColumnBuilders;
type KalamTableWithOptions = {
	<TTableName extends string, TColumnsMap extends ColumnMap>(
		name: TTableName,
		columns: TColumnsMap,
		options: KalamTableOptions,
	): KalamPgTableWithColumns<TTableName, TColumnsMap>;
	<TTableName extends string, TColumnsMap extends ColumnMap>(
		name: TTableName,
		columns: TColumnsMap,
		extraConfig: ExtraConfigArray<TTableName, TColumnsMap>,
		options: KalamTableOptions,
	): KalamPgTableWithColumns<TTableName, TColumnsMap>;
	<TTableName extends string, TColumnsMap extends ColumnMap>(
		name: TTableName,
		columns: TColumnsMap,
		extraConfig: ExtraConfigObject<TTableName, TColumnsMap>,
		options: KalamTableOptions,
	): KalamPgTableWithColumns<TTableName, TColumnsMap>;
};
type KalamTableKindFactory = typeof pgTable & KalamTableWithOptions;
type KalamTableFactory = KalamTableKindFactory & {
	shared: KalamTableKindFactory;
	user: KalamTableKindFactory;
	stream: KalamTableKindFactory;
	system: KalamTableKindFactory;
};

export const kalamTableConfigSymbol: unique symbol = Symbol.for('kalamdb.orm.tableConfig') as never;

const DEFAULT_VERSIONED_COLUMNS: readonly KalamSystemColumnName[] = ['_version', '_deleted'];
const STREAM_COLUMNS: readonly KalamSystemColumnName[] = ['_version'];
const ALL_SYSTEM_COLUMNS: readonly KalamSystemColumnName[] = ['_version', '_deleted', '_timestamp'];
let kalamOrmConfig: KalamOrmConfig = {};

function normalizeNamespace(namespace?: string): string | undefined {
	const trimmed = namespace?.trim();
	return trimmed ? trimmed : undefined;
}

export function configureKalamOrm(config: KalamOrmConfig): void {
	kalamOrmConfig = { namespace: normalizeNamespace(config.namespace) };
}

export function getKalamOrmConfig(): Readonly<KalamOrmConfig> {
	return Object.freeze({ ...kalamOrmConfig });
}

function parseQualifiedName(qualifiedName: string): Pick<KalamTableConfig, 'namespace' | 'name' | 'tableId'> {
	const [namespace, ...tableParts] = qualifiedName.split('.');
	if (tableParts.length === 0) {
		return { name: qualifiedName, tableId: qualifiedName };
	}

	const tableName = tableParts.join('.');
	return { namespace, name: tableName, tableId: `${namespace}:${tableName}` };
}

function resolveTableName(inputName: string): KalamTableConfig {
	const trimmedName = inputName.trim();
	const parsedName = parseQualifiedName(trimmedName);
	const namespace = parsedName.namespace ?? kalamOrmConfig.namespace;
	const name = parsedName.name;

	return {
		qualifiedName: namespace ? `${namespace}.${name}` : name,
		namespace,
		name,
		tableId: namespace ? `${namespace}:${name}` : name,
		systemColumns: [],
	};
}

function normalizeSystemColumns(options: KalamTableOptions): readonly KalamSystemColumnName[] {
	const requested = options.systemColumns;
	if (requested === false || requested === undefined) return [];
	if (requested === 'all') return ALL_SYSTEM_COLUMNS;
	if (Array.isArray(requested)) return requested;
	if (options.tableType === 'stream') return STREAM_COLUMNS;
	if (options.tableType === 'system') return [];
	return DEFAULT_VERSIONED_COLUMNS;
}

function buildSystemColumns(names: readonly KalamSystemColumnName[]): ColumnMap {
	const columns: ColumnMap = {};

	for (const name of names) {
		if (name === '_deleted') {
			columns._deleted = boolean('_deleted');
		} else {
			columns[name] = text(name);
		}
	}

	return columns;
}

function mergeSystemColumns(columns: ColumnMap, systemColumns: readonly KalamSystemColumnName[]): ColumnMap {
	if (systemColumns.length === 0) return columns;

	const merged: ColumnMap = { ...columns };
	for (const [name, builder] of Object.entries(buildSystemColumns(systemColumns))) {
		if (!(name in merged)) merged[name] = builder;
	}

	return merged;
}

function withSystemColumns(columns: ColumnsInput, systemColumns: readonly KalamSystemColumnName[]): ColumnsInput {
	if (typeof columns !== 'function') {
		return mergeSystemColumns(columns, systemColumns);
	}

	return (columnTypes: unknown) => mergeSystemColumns(columns(columnTypes), systemColumns);
}

function attachConfig<TTable extends Table>(table: TTable, qualifiedName: string, options: KalamTableOptions): TTable {
	const parsedName = parseQualifiedName(qualifiedName);
	const config: KalamTableConfig = {
		qualifiedName,
		...parsedName,
		tableType: options.tableType,
		systemColumns: normalizeSystemColumns(options),
	};

	Object.defineProperty(table, kalamTableConfigSymbol, {
		value: Object.freeze(config),
		enumerable: false,
	});

	return table;
}

function isKalamOptions(value: unknown): value is KalamTableOptions {
	return typeof value === 'object' && value !== null && ('tableType' in value || 'systemColumns' in value);
}

function createKTable(defaultTableType?: KalamTableType) {
	return (name: string, columns: ColumnsInput, third?: ExtraConfig | KalamTableOptions, fourth?: KalamTableOptions) => {
		const hasExtraConfig = typeof third === 'function';
		const options = {
			tableType: defaultTableType,
			...(isKalamOptions(third) ? third : undefined),
			...fourth,
		} satisfies KalamTableOptions;
		const resolvedTable = resolveTableName(name);
		const systemColumns = normalizeSystemColumns(options);
		const resolvedColumns = withSystemColumns(columns, systemColumns);
		// Drizzle rejects pgSchema("public"); its default schema is already public.
		const drizzleNamespace = resolvedTable.namespace && resolvedTable.namespace.toLowerCase() !== 'public'
			? resolvedTable.namespace
			: undefined;
		const table = drizzleNamespace
			? hasExtraConfig
				? pgSchema(drizzleNamespace).table(resolvedTable.name, resolvedColumns as never, third as never)
				: pgSchema(drizzleNamespace).table(resolvedTable.name, resolvedColumns as never)
			: hasExtraConfig
				? pgTable(resolvedTable.name, resolvedColumns as never, third as never)
				: pgTable(resolvedTable.name, resolvedColumns as never);

		return attachConfig(table, resolvedTable.qualifiedName, options);
	};
}

export function getKalamTableConfig(table: Table): KalamTableConfig | undefined {
	return (table as Table & { [kalamTableConfigSymbol]?: KalamTableConfig })[kalamTableConfigSymbol];
}

export function kSystemColumns(): PublicVersionSystemColumnBuilders;
export function kSystemColumns(names: readonly ['_version']): VersionSystemColumnBuilders;
export function kSystemColumns(names: readonly ['_version', '_deleted']): PublicVersionSystemColumnBuilders;
export function kSystemColumns(names: readonly ['_version', '_deleted', '_timestamp']): AllSystemColumnBuilders;
export function kSystemColumns(names: readonly KalamSystemColumnName[]): ColumnMap;
export function kSystemColumns(names: readonly KalamSystemColumnName[] = DEFAULT_VERSIONED_COLUMNS): ColumnMap {
	return buildSystemColumns(names);
}

const tableFactory = createKTable() as unknown as KalamTableFactory;
tableFactory.shared = createKTable('shared') as unknown as KalamTableKindFactory;
tableFactory.user = createKTable('user') as unknown as KalamTableKindFactory;
tableFactory.stream = createKTable('stream') as unknown as KalamTableKindFactory;
tableFactory.system = createKTable('system') as unknown as KalamTableKindFactory;

export const kTable = tableFactory;
