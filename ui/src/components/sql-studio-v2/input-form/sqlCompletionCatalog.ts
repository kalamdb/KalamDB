import type { StudioNamespace } from "../shared/types";
import type { ProcedureMetadata } from "@/features/functions/types";
import { procedureCompletionEntries } from "@/features/functions/sqlCompletions";

type CompletionCategory = "function" | "keyword" | "operator" | "snippet" | "type";

export interface SqlCompletionEntry {
  label: string;
  detail: string;
  category: CompletionCategory;
  insertText?: string;
  isSnippet?: boolean;
  sortText?: string;
}

export interface SqlCompletionData {
  namespaces: string[];
  tablesByNamespace: Record<string, string[]>;
  columnsByTable: Record<string, string[]>;
  keywords: readonly string[];
  functions: readonly SqlCompletionEntry[];
  snippets: readonly SqlCompletionEntry[];
  types: readonly SqlCompletionEntry[];
  operators: readonly SqlCompletionEntry[];
  procedures: ProcedureMetadata[];
  procedureEntries: readonly SqlCompletionEntry[];
}

export interface SqlContextualCompletionMatch {
  kind: "table" | "column";
  labels: string[];
  detail: string;
  partial: string;
}

const STATIC_SYSTEM_TABLES = [
  {
    namespace: "system",
    name: "live",
    columns: [
      "live_id",
      "connection_id",
      "subscription_id",
      "namespace_id",
      "table_name",
      "user_id",
      "query",
      "options",
      "status",
      "created_at",
      "last_update",
      "changes",
      "node_id",
      "last_ping_at",
    ],
  },
] as const;

const SQL_KEYWORDS = [
  "ACK",
  "ADD",
  "ALL",
  "ALTER",
  "ANALYZE",
  "AND",
  "AS",
  "ASC",
  "BACKUP",
  "BEGIN",
  "BETWEEN",
  "BY",
  "CALL",
  "CASE",
  "CHECK",
  "CLEAR",
  "CLUSTER",
  "COMMIT",
  "COMPACT",
  "CONSUME",
  "CREATE",
  "CROSS",
  "DATABASE",
  "DEFAULT",
  "DELETE",
  "DESC",
  "DESCRIBE",
  "DISTINCT",
  "DROP",
  "ELSE",
  "END",
  "EXECUTE",
  "EXISTS",
  "EXPLAIN",
  "EXPORT",
  "FALSE",
  "FLUSH",
  "FOLLOWING",
  "FOR",
  "FROM",
  "FULL",
  "GROUP BY",
  "HAVING",
  "IF",
  "IN",
  "INNER JOIN",
  "INSERT INTO",
  "IS",
  "JOIN",
  "KEY",
  "KILL",
  "LAST",
  "LEFT JOIN",
  "LIKE",
  "LIMIT",
  "LIVE",
  "NAMESPACE",
  "NOT",
  "NULL",
  "OFFSET",
  "ON",
  "OPTIONS",
  "OR",
  "ORDER BY",
  "OUTER",
  "OVER",
  "PARTITION BY",
  "PRECEDING",
  "PRIMARY KEY",
  "QUERY",
  "RANGE",
  "RESTORE",
  "RIGHT JOIN",
  "ROLLBACK",
  "ROW",
  "ROWS",
  "SELECT",
  "SET",
  "SHARED",
  "SHOW",
  "START TRANSACTION",
  "STORAGE",
  "STREAM",
  "SUBSCRIBE TO",
  "TABLE",
  "THEN",
  "TO",
  "TOPIC",
  "TRUE",
  "UNBOUNDED",
  "UNION",
  "UPDATE",
  "USE",
  "USER",
  "VALUES",
  "VIEW",
  "WHEN",
  "WHERE",
  "WITH",
] as const;

const SQL_TYPE_NAMES = [
  "ARRAY",
  "BIGINT",
  "BOOLEAN",
  "DATE",
  "DOUBLE",
  "FLOAT",
  "INT",
  "INTEGER",
  "JSON",
  "JSONB",
  "LIST",
  "MAP",
  "REAL",
  "SMALLINT",
  "STRUCT",
  "TEXT",
  "TIME",
  "TIMESTAMP",
  "TIMESTAMPTZ",
  "UUID",
  "VARCHAR",
] as const;

function functionEntry(name: string, detail: string, insertText?: string): SqlCompletionEntry {
  return {
    label: `${name}()`,
    detail,
    category: "function",
    insertText: insertText ?? `${name}()`,
    isSnippet: Boolean(insertText),
    sortText: `1_${name}`,
  };
}

function syntaxEntry(label: string, detail: string, insertText?: string): SqlCompletionEntry {
  return {
    label,
    detail,
    category: "snippet",
    insertText: insertText ?? label,
    isSnippet: Boolean(insertText),
    sortText: `0_${label}`,
  };
}

function typeEntry(label: string): SqlCompletionEntry {
  return {
    label,
    detail: "SQL type",
    category: "type",
    sortText: `3_${label}`,
  };
}

function operatorEntry(label: string, detail: string): SqlCompletionEntry {
  return {
    label,
    detail,
    category: "operator",
    sortText: `4_${label}`,
  };
}

const KALAMDB_FUNCTION_COMPLETIONS = [
  functionEntry("CURRENT_USER", "KalamDB context function"),
  functionEntry("CURRENT_USER_ID", "KalamDB context function"),
  functionEntry("CURRENT_ROLE", "KalamDB context function"),
  functionEntry("SNOWFLAKE_ID", "KalamDB ID generation function"),
  functionEntry("UUID_V7", "KalamDB ID generation function"),
  functionEntry("ULID", "KalamDB ID generation function"),
  functionEntry("COSINE_DISTANCE", "KalamDB vector similarity function", "COSINE_DISTANCE(${1:vector}, ${2:query_vector})"),
  functionEntry("VECTOR_SEARCH", "KalamDB vector table function", "VECTOR_SEARCH(${1:table}, ${2:query_vector})"),
] as const;

const DATAFUSION_SCALAR_FUNCTION_COMPLETIONS = [
  functionEntry("ABS", "DataFusion math function", "ABS(${1:number})"),
  functionEntry("ACOS", "DataFusion math function", "ACOS(${1:number})"),
  functionEntry("ACOSH", "DataFusion math function", "ACOSH(${1:number})"),
  functionEntry("ARROW_CAST", "DataFusion core function", "ARROW_CAST(${1:value}, ${2:type})"),
  functionEntry("ARROW_METADATA", "DataFusion core function", "ARROW_METADATA(${1:value})"),
  functionEntry("ARROW_TYPEOF", "DataFusion core function", "ARROW_TYPEOF(${1:value})"),
  functionEntry("ASCII", "DataFusion string function", "ASCII(${1:string})"),
  functionEntry("ASIN", "DataFusion math function", "ASIN(${1:number})"),
  functionEntry("ASINH", "DataFusion math function", "ASINH(${1:number})"),
  functionEntry("ATAN", "DataFusion math function", "ATAN(${1:number})"),
  functionEntry("ATAN2", "DataFusion math function", "ATAN2(${1:y}, ${2:x})"),
  functionEntry("ATANH", "DataFusion math function", "ATANH(${1:number})"),
  functionEntry("BIT_LENGTH", "DataFusion string function", "BIT_LENGTH(${1:string})"),
  functionEntry("BTRIM", "DataFusion string function", "BTRIM(${1:string})"),
  functionEntry("CBRT", "DataFusion math function", "CBRT(${1:number})"),
  functionEntry("CEIL", "DataFusion math function", "CEIL(${1:number})"),
  functionEntry("CHR", "DataFusion string function", "CHR(${1:code_point})"),
  functionEntry("COALESCE", "DataFusion core function", "COALESCE(${1:value}, ${2:fallback})"),
  functionEntry("CONCAT", "DataFusion string function", "CONCAT(${1:value})"),
  functionEntry("CONCAT_WS", "DataFusion string function", "CONCAT_WS(${1:separator}, ${2:value})"),
  functionEntry("CONTAINS", "DataFusion string function", "CONTAINS(${1:string}, ${2:search_string})"),
  functionEntry("COS", "DataFusion math function", "COS(${1:number})"),
  functionEntry("COSH", "DataFusion math function", "COSH(${1:number})"),
  functionEntry("COT", "DataFusion math function", "COT(${1:number})"),
  functionEntry("CURRENT_DATE", "DataFusion datetime function"),
  functionEntry("CURRENT_TIME", "DataFusion datetime function"),
  functionEntry("CURRENT_TIMESTAMP", "DataFusion datetime function"),
  functionEntry("DATE_BIN", "DataFusion datetime function", "DATE_BIN(${1:stride}, ${2:source}, ${3:origin})"),
  functionEntry("DATE_PART", "DataFusion datetime function", "DATE_PART(${1:part}, ${2:date})"),
  functionEntry("DATE_TRUNC", "DataFusion datetime function", "DATE_TRUNC(${1:part}, ${2:date})"),
  functionEntry("DEGREES", "DataFusion math function", "DEGREES(${1:number})"),
  functionEntry("DIGEST", "DataFusion crypto function", "DIGEST(${1:value}, ${2:algorithm})"),
  functionEntry("ENDS_WITH", "DataFusion string function", "ENDS_WITH(${1:string}, ${2:suffix})"),
  functionEntry("EXP", "DataFusion math function", "EXP(${1:number})"),
  functionEntry("FACTORIAL", "DataFusion math function", "FACTORIAL(${1:number})"),
  functionEntry("FLOOR", "DataFusion math function", "FLOOR(${1:number})"),
  functionEntry("FROM_UNIXTIME", "DataFusion datetime function", "FROM_UNIXTIME(${1:seconds})"),
  functionEntry("GCD", "DataFusion math function", "GCD(${1:x}, ${2:y})"),
  functionEntry("GET_FIELD", "DataFusion core function", "GET_FIELD(${1:value}, ${2:field})"),
  functionEntry("GREATEST", "DataFusion core function", "GREATEST(${1:value})"),
  functionEntry("ISNAN", "DataFusion math function", "ISNAN(${1:number})"),
  functionEntry("ISZERO", "DataFusion math function", "ISZERO(${1:number})"),
  functionEntry("LCM", "DataFusion math function", "LCM(${1:x}, ${2:y})"),
  functionEntry("LEAST", "DataFusion core function", "LEAST(${1:value})"),
  functionEntry("LEFT", "DataFusion unicode function", "LEFT(${1:string}, ${2:n})"),
  functionEntry("LENGTH", "DataFusion string/unicode function", "LENGTH(${1:string})"),
  functionEntry("LEVENSHTEIN", "DataFusion string function", "LEVENSHTEIN(${1:left}, ${2:right})"),
  functionEntry("LN", "DataFusion math function", "LN(${1:number})"),
  functionEntry("LOG", "DataFusion math function", "LOG(${1:base}, ${2:number})"),
  functionEntry("LOG2", "DataFusion math function", "LOG2(${1:number})"),
  functionEntry("LOG10", "DataFusion math function", "LOG10(${1:number})"),
  functionEntry("LOWER", "DataFusion string function", "LOWER(${1:string})"),
  functionEntry("LPAD", "DataFusion unicode function", "LPAD(${1:string}, ${2:length}, ${3:fill})"),
  functionEntry("LTRIM", "DataFusion string function", "LTRIM(${1:string})"),
  functionEntry("MAKE_DATE", "DataFusion datetime function", "MAKE_DATE(${1:year}, ${2:month}, ${3:day})"),
  functionEntry("MAKE_TIME", "DataFusion datetime function", "MAKE_TIME(${1:hour}, ${2:minute}, ${3:second})"),
  functionEntry("MD5", "DataFusion crypto function", "MD5(${1:value})"),
  functionEntry("NANVL", "DataFusion math function", "NANVL(${1:x}, ${2:y})"),
  functionEntry("NAMED_STRUCT", "DataFusion core function", "NAMED_STRUCT(${1:name}, ${2:value})"),
  functionEntry("NOW", "DataFusion datetime function"),
  functionEntry("NULLIF", "DataFusion core function", "NULLIF(${1:left}, ${2:right})"),
  functionEntry("NVL", "DataFusion core function", "NVL(${1:value}, ${2:fallback})"),
  functionEntry("NVL2", "DataFusion core function", "NVL2(${1:value}, ${2:not_null}, ${3:null_value})"),
  functionEntry("OCTET_LENGTH", "DataFusion string function", "OCTET_LENGTH(${1:string})"),
  functionEntry("OVERLAY", "DataFusion core/string function", "OVERLAY(${1:string} PLACING ${2:replacement} FROM ${3:start})"),
  functionEntry("PI", "DataFusion math function"),
  functionEntry("POWER", "DataFusion math function", "POWER(${1:base}, ${2:exponent})"),
  functionEntry("RADIANS", "DataFusion math function", "RADIANS(${1:number})"),
  functionEntry("RANDOM", "DataFusion math function"),
  functionEntry("REGEXP_COUNT", "DataFusion regex function", "REGEXP_COUNT(${1:string}, ${2:pattern})"),
  functionEntry("REGEXP_INSTR", "DataFusion regex function", "REGEXP_INSTR(${1:string}, ${2:pattern})"),
  functionEntry("REGEXP_LIKE", "DataFusion regex function", "REGEXP_LIKE(${1:string}, ${2:pattern})"),
  functionEntry("REGEXP_MATCH", "DataFusion regex function", "REGEXP_MATCH(${1:string}, ${2:pattern})"),
  functionEntry("REGEXP_REPLACE", "DataFusion regex function", "REGEXP_REPLACE(${1:string}, ${2:pattern}, ${3:replacement})"),
  functionEntry("REPEAT", "DataFusion string function", "REPEAT(${1:string}, ${2:n})"),
  functionEntry("REPLACE", "DataFusion string function", "REPLACE(${1:string}, ${2:from}, ${3:to})"),
  functionEntry("REVERSE", "DataFusion unicode function", "REVERSE(${1:string})"),
  functionEntry("RIGHT", "DataFusion unicode function", "RIGHT(${1:string}, ${2:n})"),
  functionEntry("ROUND", "DataFusion math function", "ROUND(${1:number})"),
  functionEntry("RPAD", "DataFusion unicode function", "RPAD(${1:string}, ${2:length}, ${3:fill})"),
  functionEntry("RTRIM", "DataFusion string function", "RTRIM(${1:string})"),
  functionEntry("SHA224", "DataFusion crypto function", "SHA224(${1:value})"),
  functionEntry("SHA256", "DataFusion crypto function", "SHA256(${1:value})"),
  functionEntry("SHA384", "DataFusion crypto function", "SHA384(${1:value})"),
  functionEntry("SHA512", "DataFusion crypto function", "SHA512(${1:value})"),
  functionEntry("SIGNUM", "DataFusion math function", "SIGNUM(${1:number})"),
  functionEntry("SIN", "DataFusion math function", "SIN(${1:number})"),
  functionEntry("SINH", "DataFusion math function", "SINH(${1:number})"),
  functionEntry("SPLIT_PART", "DataFusion string function", "SPLIT_PART(${1:string}, ${2:delimiter}, ${3:index})"),
  functionEntry("SQRT", "DataFusion math function", "SQRT(${1:number})"),
  functionEntry("STARTS_WITH", "DataFusion string function", "STARTS_WITH(${1:string}, ${2:prefix})"),
  functionEntry("STRPOS", "DataFusion unicode function", "STRPOS(${1:string}, ${2:substring})"),
  functionEntry("STRUCT", "DataFusion core function", "STRUCT(${1:value})"),
  functionEntry("SUBSTR", "DataFusion unicode function", "SUBSTR(${1:string}, ${2:start})"),
  functionEntry("TAN", "DataFusion math function", "TAN(${1:number})"),
  functionEntry("TANH", "DataFusion math function", "TANH(${1:number})"),
  functionEntry("TO_CHAR", "DataFusion datetime function", "TO_CHAR(${1:value}, ${2:format})"),
  functionEntry("TO_DATE", "DataFusion datetime function", "TO_DATE(${1:value})"),
  functionEntry("TO_HEX", "DataFusion string function", "TO_HEX(${1:value})"),
  functionEntry("TO_LOCAL_TIME", "DataFusion datetime function", "TO_LOCAL_TIME(${1:value})"),
  functionEntry("TO_TIME", "DataFusion datetime function", "TO_TIME(${1:value})"),
  functionEntry("TO_TIMESTAMP", "DataFusion datetime function", "TO_TIMESTAMP(${1:value})"),
  functionEntry("TO_TIMESTAMP_SECONDS", "DataFusion datetime function", "TO_TIMESTAMP_SECONDS(${1:value})"),
  functionEntry("TO_TIMESTAMP_MILLIS", "DataFusion datetime function", "TO_TIMESTAMP_MILLIS(${1:value})"),
  functionEntry("TO_TIMESTAMP_MICROS", "DataFusion datetime function", "TO_TIMESTAMP_MICROS(${1:value})"),
  functionEntry("TO_TIMESTAMP_NANOS", "DataFusion datetime function", "TO_TIMESTAMP_NANOS(${1:value})"),
  functionEntry("TO_UNIXTIME", "DataFusion datetime function", "TO_UNIXTIME(${1:value})"),
  functionEntry("TRANSLATE", "DataFusion unicode function", "TRANSLATE(${1:string}, ${2:from}, ${3:to})"),
  functionEntry("TRIM", "DataFusion string function", "TRIM(${1:string})"),
  functionEntry("TRUNC", "DataFusion math function", "TRUNC(${1:number})"),
  functionEntry("UNION_EXTRACT", "DataFusion core function", "UNION_EXTRACT(${1:value}, ${2:field})"),
  functionEntry("UNION_TAG", "DataFusion core function", "UNION_TAG(${1:value})"),
  functionEntry("UPPER", "DataFusion string function", "UPPER(${1:string})"),
  functionEntry("UUID", "DataFusion string function"),
  functionEntry("VERSION", "DataFusion core function"),
] as const;

const DATAFUSION_AGGREGATE_FUNCTION_COMPLETIONS = [
  functionEntry("APPROX_DISTINCT", "DataFusion aggregate function", "APPROX_DISTINCT(${1:expression})"),
  functionEntry("APPROX_MEDIAN", "DataFusion aggregate function", "APPROX_MEDIAN(${1:expression})"),
  functionEntry("APPROX_PERCENTILE_CONT", "DataFusion aggregate function", "APPROX_PERCENTILE_CONT(${1:expression}, ${2:percentile})"),
  functionEntry("APPROX_PERCENTILE_CONT_WITH_WEIGHT", "DataFusion aggregate function", "APPROX_PERCENTILE_CONT_WITH_WEIGHT(${1:expression}, ${2:weight}, ${3:percentile})"),
  functionEntry("ARRAY_AGG", "DataFusion aggregate function", "ARRAY_AGG(${1:expression})"),
  functionEntry("AVG", "DataFusion aggregate function", "AVG(${1:expression})"),
  functionEntry("BIT_AND", "DataFusion aggregate function", "BIT_AND(${1:expression})"),
  functionEntry("BIT_OR", "DataFusion aggregate function", "BIT_OR(${1:expression})"),
  functionEntry("BIT_XOR", "DataFusion aggregate function", "BIT_XOR(${1:expression})"),
  functionEntry("BOOL_AND", "DataFusion aggregate function", "BOOL_AND(${1:expression})"),
  functionEntry("BOOL_OR", "DataFusion aggregate function", "BOOL_OR(${1:expression})"),
  functionEntry("CORR", "DataFusion aggregate function", "CORR(${1:y}, ${2:x})"),
  functionEntry("COUNT", "DataFusion aggregate function", "COUNT(${1:*})"),
  functionEntry("COVAR_POP", "DataFusion aggregate function", "COVAR_POP(${1:y}, ${2:x})"),
  functionEntry("COVAR_SAMP", "DataFusion aggregate function", "COVAR_SAMP(${1:y}, ${2:x})"),
  functionEntry("GROUPING", "DataFusion aggregate function", "GROUPING(${1:expression})"),
  functionEntry("MAX", "DataFusion aggregate function", "MAX(${1:expression})"),
  functionEntry("MEDIAN", "DataFusion aggregate function", "MEDIAN(${1:expression})"),
  functionEntry("MIN", "DataFusion aggregate function", "MIN(${1:expression})"),
  functionEntry("NTH_VALUE", "DataFusion aggregate/window function", "NTH_VALUE(${1:expression}, ${2:n})"),
  functionEntry("PERCENTILE_CONT", "DataFusion aggregate function", "PERCENTILE_CONT(${1:expression}, ${2:percentile})"),
  functionEntry("REGR_AVGX", "DataFusion regression aggregate", "REGR_AVGX(${1:y}, ${2:x})"),
  functionEntry("REGR_AVGY", "DataFusion regression aggregate", "REGR_AVGY(${1:y}, ${2:x})"),
  functionEntry("REGR_COUNT", "DataFusion regression aggregate", "REGR_COUNT(${1:y}, ${2:x})"),
  functionEntry("REGR_INTERCEPT", "DataFusion regression aggregate", "REGR_INTERCEPT(${1:y}, ${2:x})"),
  functionEntry("REGR_R2", "DataFusion regression aggregate", "REGR_R2(${1:y}, ${2:x})"),
  functionEntry("REGR_SLOPE", "DataFusion regression aggregate", "REGR_SLOPE(${1:y}, ${2:x})"),
  functionEntry("REGR_SXX", "DataFusion regression aggregate", "REGR_SXX(${1:y}, ${2:x})"),
  functionEntry("REGR_SXY", "DataFusion regression aggregate", "REGR_SXY(${1:y}, ${2:x})"),
  functionEntry("REGR_SYY", "DataFusion regression aggregate", "REGR_SYY(${1:y}, ${2:x})"),
  functionEntry("STDDEV", "DataFusion aggregate function", "STDDEV(${1:expression})"),
  functionEntry("STDDEV_POP", "DataFusion aggregate function", "STDDEV_POP(${1:expression})"),
  functionEntry("STRING_AGG", "DataFusion aggregate function", "STRING_AGG(${1:expression}, ${2:delimiter})"),
  functionEntry("SUM", "DataFusion aggregate function", "SUM(${1:expression})"),
  functionEntry("VAR_POP", "DataFusion aggregate function", "VAR_POP(${1:expression})"),
  functionEntry("VAR_SAMPLE", "DataFusion aggregate function", "VAR_SAMPLE(${1:expression})"),
] as const;

const DATAFUSION_WINDOW_FUNCTION_COMPLETIONS = [
  functionEntry("CUME_DIST", "DataFusion window function"),
  functionEntry("DENSE_RANK", "DataFusion window function"),
  functionEntry("FIRST_VALUE", "DataFusion window function", "FIRST_VALUE(${1:expression})"),
  functionEntry("LAG", "DataFusion window function", "LAG(${1:expression})"),
  functionEntry("LAST_VALUE", "DataFusion window function", "LAST_VALUE(${1:expression})"),
  functionEntry("LEAD", "DataFusion window function", "LEAD(${1:expression})"),
  functionEntry("NTILE", "DataFusion window function", "NTILE(${1:buckets})"),
  functionEntry("PERCENT_RANK", "DataFusion window function"),
  functionEntry("RANK", "DataFusion window function"),
  functionEntry("ROW_NUMBER", "DataFusion window function"),
] as const;

const DATAFUSION_NESTED_FUNCTION_COMPLETIONS = [
  functionEntry("ARRAY_ANY_VALUE", "DataFusion nested function", "ARRAY_ANY_VALUE(${1:array})"),
  functionEntry("ARRAY_APPEND", "DataFusion nested function", "ARRAY_APPEND(${1:array}, ${2:value})"),
  functionEntry("ARRAY_CONCAT", "DataFusion nested function", "ARRAY_CONCAT(${1:array})"),
  functionEntry("ARRAY_DIMS", "DataFusion nested function", "ARRAY_DIMS(${1:array})"),
  functionEntry("ARRAY_DISTINCT", "DataFusion nested function", "ARRAY_DISTINCT(${1:array})"),
  functionEntry("ARRAY_DISTANCE", "DataFusion nested function", "ARRAY_DISTANCE(${1:left}, ${2:right})"),
  functionEntry("ARRAY_ELEMENT", "DataFusion nested function", "ARRAY_ELEMENT(${1:array}, ${2:index})"),
  functionEntry("ARRAY_EMPTY", "DataFusion nested function", "ARRAY_EMPTY(${1:array})"),
  functionEntry("ARRAY_EXCEPT", "DataFusion nested function", "ARRAY_EXCEPT(${1:left}, ${2:right})"),
  functionEntry("ARRAY_HAS", "DataFusion nested function", "ARRAY_HAS(${1:array}, ${2:value})"),
  functionEntry("ARRAY_HAS_ALL", "DataFusion nested function", "ARRAY_HAS_ALL(${1:array}, ${2:values})"),
  functionEntry("ARRAY_HAS_ANY", "DataFusion nested function", "ARRAY_HAS_ANY(${1:array}, ${2:values})"),
  functionEntry("ARRAY_INTERSECT", "DataFusion nested function", "ARRAY_INTERSECT(${1:left}, ${2:right})"),
  functionEntry("ARRAY_LENGTH", "DataFusion nested function", "ARRAY_LENGTH(${1:array})"),
  functionEntry("ARRAY_MAX", "DataFusion nested function", "ARRAY_MAX(${1:array})"),
  functionEntry("ARRAY_MIN", "DataFusion nested function", "ARRAY_MIN(${1:array})"),
  functionEntry("ARRAY_NDIMS", "DataFusion nested function", "ARRAY_NDIMS(${1:array})"),
  functionEntry("ARRAY_POP_BACK", "DataFusion nested function", "ARRAY_POP_BACK(${1:array})"),
  functionEntry("ARRAY_POP_FRONT", "DataFusion nested function", "ARRAY_POP_FRONT(${1:array})"),
  functionEntry("ARRAY_POSITION", "DataFusion nested function", "ARRAY_POSITION(${1:array}, ${2:value})"),
  functionEntry("ARRAY_POSITIONS", "DataFusion nested function", "ARRAY_POSITIONS(${1:array}, ${2:value})"),
  functionEntry("ARRAY_PREPEND", "DataFusion nested function", "ARRAY_PREPEND(${1:value}, ${2:array})"),
  functionEntry("ARRAY_REMOVE", "DataFusion nested function", "ARRAY_REMOVE(${1:array}, ${2:value})"),
  functionEntry("ARRAY_REMOVE_ALL", "DataFusion nested function", "ARRAY_REMOVE_ALL(${1:array}, ${2:value})"),
  functionEntry("ARRAY_REMOVE_N", "DataFusion nested function", "ARRAY_REMOVE_N(${1:array}, ${2:value}, ${3:n})"),
  functionEntry("ARRAY_REPEAT", "DataFusion nested function", "ARRAY_REPEAT(${1:value}, ${2:n})"),
  functionEntry("ARRAY_REPLACE", "DataFusion nested function", "ARRAY_REPLACE(${1:array}, ${2:from}, ${3:to})"),
  functionEntry("ARRAY_REPLACE_ALL", "DataFusion nested function", "ARRAY_REPLACE_ALL(${1:array}, ${2:from}, ${3:to})"),
  functionEntry("ARRAY_REPLACE_N", "DataFusion nested function", "ARRAY_REPLACE_N(${1:array}, ${2:from}, ${3:to}, ${4:n})"),
  functionEntry("ARRAY_RESIZE", "DataFusion nested function", "ARRAY_RESIZE(${1:array}, ${2:size})"),
  functionEntry("ARRAY_REVERSE", "DataFusion nested function", "ARRAY_REVERSE(${1:array})"),
  functionEntry("ARRAY_SLICE", "DataFusion nested function", "ARRAY_SLICE(${1:array}, ${2:start}, ${3:end})"),
  functionEntry("ARRAY_SORT", "DataFusion nested function", "ARRAY_SORT(${1:array})"),
  functionEntry("ARRAY_TO_STRING", "DataFusion nested function", "ARRAY_TO_STRING(${1:array}, ${2:delimiter})"),
  functionEntry("ARRAY_UNION", "DataFusion nested function", "ARRAY_UNION(${1:left}, ${2:right})"),
  functionEntry("ARRAYS_ZIP", "DataFusion nested function", "ARRAYS_ZIP(${1:array})"),
  functionEntry("CARDINALITY", "DataFusion nested function", "CARDINALITY(${1:value})"),
  functionEntry("FLATTEN", "DataFusion nested function", "FLATTEN(${1:array})"),
  functionEntry("GEN_SERIES", "DataFusion nested function", "GEN_SERIES(${1:start}, ${2:stop})"),
  functionEntry("MAKE_ARRAY", "DataFusion nested function", "MAKE_ARRAY(${1:value})"),
  functionEntry("MAP", "DataFusion nested function", "MAP(${1:key}, ${2:value})"),
  functionEntry("MAP_ENTRIES", "DataFusion nested function", "MAP_ENTRIES(${1:map})"),
  functionEntry("MAP_EXTRACT", "DataFusion nested function", "MAP_EXTRACT(${1:map}, ${2:key})"),
  functionEntry("MAP_KEYS", "DataFusion nested function", "MAP_KEYS(${1:map})"),
  functionEntry("MAP_VALUES", "DataFusion nested function", "MAP_VALUES(${1:map})"),
  functionEntry("RANGE", "DataFusion nested function", "RANGE(${1:start}, ${2:stop})"),
  functionEntry("STRING_TO_ARRAY", "DataFusion nested function", "STRING_TO_ARRAY(${1:string}, ${2:delimiter})"),
] as const;

const DATAFUSION_JSON_FUNCTION_COMPLETIONS = [
  functionEntry("JSON_AS_TEXT", "DataFusion JSON function", "JSON_AS_TEXT(${1:json}, ${2:path})"),
  functionEntry("JSON_CONTAINS", "DataFusion JSON function", "JSON_CONTAINS(${1:json}, ${2:key})"),
  functionEntry("JSON_FROM_SCALAR", "DataFusion JSON function", "JSON_FROM_SCALAR(${1:value})"),
  functionEntry("JSON_GET", "DataFusion JSON function", "JSON_GET(${1:json}, ${2:path})"),
  functionEntry("JSON_GET_ARRAY", "DataFusion JSON function", "JSON_GET_ARRAY(${1:json}, ${2:path})"),
  functionEntry("JSON_GET_BOOL", "DataFusion JSON function", "JSON_GET_BOOL(${1:json}, ${2:path})"),
  functionEntry("JSON_GET_FLOAT", "DataFusion JSON function", "JSON_GET_FLOAT(${1:json}, ${2:path})"),
  functionEntry("JSON_GET_INT", "DataFusion JSON function", "JSON_GET_INT(${1:json}, ${2:path})"),
  functionEntry("JSON_GET_JSON", "DataFusion JSON function", "JSON_GET_JSON(${1:json}, ${2:path})"),
  functionEntry("JSON_GET_STR", "DataFusion JSON function", "JSON_GET_STR(${1:json}, ${2:path})"),
  functionEntry("JSON_LENGTH", "DataFusion JSON function", "JSON_LENGTH(${1:json})"),
  functionEntry("JSON_OBJECT_KEYS", "DataFusion JSON function", "JSON_OBJECT_KEYS(${1:json})"),
] as const;

const SQL_FUNCTION_COMPLETIONS = [
  ...KALAMDB_FUNCTION_COMPLETIONS,
  ...DATAFUSION_SCALAR_FUNCTION_COMPLETIONS,
  ...DATAFUSION_AGGREGATE_FUNCTION_COMPLETIONS,
  ...DATAFUSION_WINDOW_FUNCTION_COMPLETIONS,
  ...DATAFUSION_NESTED_FUNCTION_COMPLETIONS,
  ...DATAFUSION_JSON_FUNCTION_COMPLETIONS,
] as const;

const SQL_SYNTAX_COMPLETIONS = [
  syntaxEntry("SELECT", "SQL query", "SELECT ${1:*}\nFROM ${2:table}\nWHERE ${3:condition};"),
  syntaxEntry("WITH", "SQL common table expression", "WITH ${1:cte} AS (\n  SELECT ${2:*}\n  FROM ${3:table}\n)\nSELECT * FROM ${1:cte};"),
  syntaxEntry("INSERT INTO", "SQL insert", "INSERT INTO ${1:table} (${2:columns})\nVALUES (${3:values});"),
  syntaxEntry("UPDATE", "SQL update", "UPDATE ${1:table}\nSET ${2:column} = ${3:value}\nWHERE ${4:condition};"),
  syntaxEntry("DELETE FROM", "SQL delete", "DELETE FROM ${1:table}\nWHERE ${2:condition};"),
  syntaxEntry("CREATE TABLE", "KalamDB table DDL", "CREATE TABLE ${1:namespace}.${2:table} (\n  ${3:id} TEXT PRIMARY KEY,\n  ${4:created_at} TIMESTAMP DEFAULT NOW()\n);"),
  syntaxEntry("CREATE USER TABLE", "KalamDB user table DDL", "CREATE USER TABLE ${1:namespace}.${2:table} (\n  ${3:id} TEXT PRIMARY KEY\n);"),
  syntaxEntry("CREATE SHARED TABLE", "KalamDB shared table DDL", "CREATE SHARED TABLE ${1:namespace}.${2:table} (\n  ${3:id} TEXT PRIMARY KEY\n);"),
  syntaxEntry("CREATE STREAM TABLE", "KalamDB stream table DDL", "CREATE STREAM TABLE ${1:namespace}.${2:table} (\n  ${3:id} TEXT PRIMARY KEY\n);"),
  syntaxEntry("ALTER TABLE", "KalamDB table DDL", "ALTER TABLE ${1:namespace}.${2:table}\nADD COLUMN ${3:column} ${4:TEXT};"),
  syntaxEntry("DROP TABLE", "KalamDB table DDL", "DROP TABLE ${1:namespace}.${2:table};"),
  syntaxEntry("CREATE VIEW", "KalamDB view DDL", "CREATE VIEW ${1:namespace}.${2:view_name} AS\nSELECT ${3:*}\nFROM ${4:table};"),
  syntaxEntry("CREATE NAMESPACE", "KalamDB namespace DDL", "CREATE NAMESPACE ${1:namespace};"),
  syntaxEntry("ALTER NAMESPACE", "KalamDB namespace DDL", "ALTER NAMESPACE ${1:namespace} SET OPTIONS (${2:key} = '${3:value}');"),
  syntaxEntry("DROP NAMESPACE", "KalamDB namespace DDL", "DROP NAMESPACE ${1:namespace};"),
  syntaxEntry("SHOW NAMESPACES", "KalamDB metadata command", "SHOW NAMESPACES;"),
  syntaxEntry("USE NAMESPACE", "KalamDB namespace command", "USE NAMESPACE ${1:namespace};"),
  syntaxEntry("SET NAMESPACE", "KalamDB namespace command", "SET NAMESPACE ${1:namespace};"),
  syntaxEntry("SHOW TABLES", "KalamDB metadata command", "SHOW TABLES;"),
  syntaxEntry("DESCRIBE TABLE", "KalamDB metadata command", "DESCRIBE TABLE ${1:namespace}.${2:table};"),
  syntaxEntry("DESC TABLE", "KalamDB metadata command", "DESC TABLE ${1:namespace}.${2:table};"),
  syntaxEntry("SHOW STATS FOR TABLE", "KalamDB metadata command", "SHOW STATS FOR TABLE ${1:namespace}.${2:table};"),
  syntaxEntry("EXPLAIN", "DataFusion meta command", "EXPLAIN ${1:SELECT * FROM table};"),
  syntaxEntry("EXPLAIN ANALYZE", "DataFusion meta command", "EXPLAIN ANALYZE ${1:SELECT * FROM table};"),
  syntaxEntry("DESCRIBE", "DataFusion meta command", "DESCRIBE ${1:table};"),
  syntaxEntry("DESC", "DataFusion meta command", "DESC ${1:table};"),
  syntaxEntry("SHOW COLUMNS", "DataFusion meta command", "SHOW COLUMNS FROM ${1:table};"),
  syntaxEntry("SHOW ALL", "DataFusion meta command", "SHOW ALL;"),
  syntaxEntry("SET", "DataFusion session setting", "SET ${1:key} = ${2:value};"),
  syntaxEntry("EXECUTE AS USER", "KalamDB impersonation wrapper", "EXECUTE AS USER '${1:username}' (\n  ${2:SELECT * FROM table}\n);"),
  syntaxEntry("BEGIN", "KalamDB transaction control", "BEGIN;"),
  syntaxEntry("CALL", "Call a KalamDB procedure", "CALL ${1:schema.procedure}(${2:});"),
  syntaxEntry("START TRANSACTION", "KalamDB transaction control", "START TRANSACTION;"),
  syntaxEntry("COMMIT", "KalamDB transaction control", "COMMIT;"),
  syntaxEntry("ROLLBACK", "KalamDB transaction control", "ROLLBACK;"),
  syntaxEntry("SUBSCRIBE TO", "KalamDB live query subscription", "SUBSCRIBE TO ${1:namespace}.${2:table}\nWHERE ${3:user_id = CURRENT_USER()}\nOPTIONS (last_rows=${4:100});"),
  syntaxEntry("SUBSCRIBE TO SELECT", "KalamDB live query subscription", "SUBSCRIBE TO SELECT ${1:*}\nFROM ${2:namespace}.${3:table}\nWHERE ${4:user_id = CURRENT_USER()}\nOPTIONS (last_rows=${5:100}, batch_size=${6:50});"),
  syntaxEntry("KILL LIVE QUERY", "KalamDB live query command", "KILL LIVE QUERY '${1:live_id}';"),
  syntaxEntry("KILL JOB", "KalamDB job command", "KILL JOB '${1:job_id}';"),
  syntaxEntry("STORAGE FLUSH ALL", "KalamDB storage maintenance", "STORAGE FLUSH ALL;"),
  syntaxEntry("STORAGE FLUSH TABLE", "KalamDB storage maintenance", "STORAGE FLUSH TABLE ${1:namespace}.${2:table};"),
  syntaxEntry("STORAGE COMPACT ALL", "KalamDB storage maintenance", "STORAGE COMPACT ALL;"),
  syntaxEntry("STORAGE COMPACT TABLE", "KalamDB storage maintenance", "STORAGE COMPACT TABLE ${1:namespace}.${2:table};"),
  syntaxEntry("STORAGE CHECK", "KalamDB storage maintenance", "STORAGE CHECK ${1:storage_id};"),
  syntaxEntry("SHOW MANIFEST", "KalamDB storage metadata", "SHOW MANIFEST;"),
  syntaxEntry("CREATE STORAGE", "KalamDB storage DDL", "CREATE STORAGE ${1:storage_id}\nTYPE filesystem\nNAME '${2:Local Storage}'\nPATH '${3:/var/lib/kalamdb/parquet}'\nSHARED_TABLES_TEMPLATE '${4:{namespace}/{tableName}/}'\nUSER_TABLES_TEMPLATE '${5:{namespace}/{tableName}/{userId}/}';"),
  syntaxEntry("ALTER STORAGE", "KalamDB storage DDL", "ALTER STORAGE ${1:storage_id}\nSET NAME '${2:Storage Name}';"),
  syntaxEntry("DROP STORAGE", "KalamDB storage DDL", "DROP STORAGE ${1:storage_id};"),
  syntaxEntry("SHOW STORAGES", "KalamDB storage metadata", "SHOW STORAGES;"),
  syntaxEntry("CREATE TOPIC", "KalamDB topic command", "CREATE TOPIC ${1:topic_name} PARTITIONS ${2:1};"),
  syntaxEntry("DROP TOPIC", "KalamDB topic command", "DROP TOPIC ${1:topic_name};"),
  syntaxEntry("CLEAR TOPIC", "KalamDB topic command", "CLEAR TOPIC ${1:topic_name};"),
  syntaxEntry("ALTER TOPIC ADD SOURCE", "KalamDB topic command", "ALTER TOPIC ${1:topic_name}\nADD SOURCE ${2:namespace}.${3:table}\nON ${4:insert}\nWHERE ${5:condition};"),
  syntaxEntry("CONSUME FROM", "KalamDB topic command", "CONSUME FROM ${1:topic_name}\nGROUP '${2:group_id}'\nFROM ${3:latest}\nLIMIT ${4:100};"),
  syntaxEntry("ACK", "KalamDB topic command", "ACK ${1:topic_name}\nGROUP '${2:group_id}'\nPARTITION ${3:0}\nUPTO OFFSET ${4:offset};"),
  syntaxEntry("BACKUP DATABASE", "KalamDB backup command", "BACKUP DATABASE TO '${1:/backups/kalamdb.tar.gz}';"),
  syntaxEntry("RESTORE DATABASE", "KalamDB restore command", "RESTORE DATABASE FROM '${1:/backups/kalamdb.tar.gz}';"),
  syntaxEntry("EXPORT USER DATA", "KalamDB export command", "EXPORT USER DATA;"),
  syntaxEntry("SHOW EXPORT", "KalamDB export command", "SHOW EXPORT;"),
  syntaxEntry("CREATE USER", "KalamDB user management", "CREATE USER ${1:username} WITH PASSWORD '${2:password}' ROLE ${3:user};"),
  syntaxEntry("ALTER USER", "KalamDB user management", "ALTER USER ${1:username} SET ${2:ROLE} ${3:user};"),
  syntaxEntry("DROP USER", "KalamDB user management", "DROP USER ${1:username};"),
  syntaxEntry("CLUSTER SNAPSHOT", "KalamDB cluster command", "CLUSTER SNAPSHOT;"),
  syntaxEntry("CLUSTER PURGE", "KalamDB cluster command", "CLUSTER PURGE --upto ${1:index};"),
  syntaxEntry("CLUSTER TRIGGER ELECTION", "KalamDB cluster command", "CLUSTER TRIGGER ELECTION;"),
  syntaxEntry("CLUSTER TRANSFER LEADER", "KalamDB cluster command", "CLUSTER TRANSFER LEADER ${1:node_id};"),
  syntaxEntry("CLUSTER JOIN", "KalamDB cluster command", "CLUSTER JOIN ${1:node_id} RPC ${2:rpc_addr} API ${3:api_addr};"),
  syntaxEntry("CLUSTER REBALANCE", "KalamDB cluster command", "CLUSTER REBALANCE;"),
  syntaxEntry("CLUSTER STEPDOWN", "KalamDB cluster command", "CLUSTER STEPDOWN;"),
  syntaxEntry("CLUSTER CLEAR", "KalamDB cluster command", "CLUSTER CLEAR;"),
] as const;

const SQL_OPERATOR_COMPLETIONS = [
  operatorEntry("->", "JSON object operator"),
  operatorEntry("->>", "JSON text operator"),
  operatorEntry("?", "JSON contains operator"),
  operatorEntry("=", "SQL comparison operator"),
  operatorEntry("!=", "SQL comparison operator"),
  operatorEntry("<>", "SQL comparison operator"),
  operatorEntry("<=", "SQL comparison operator"),
  operatorEntry(">=", "SQL comparison operator"),
] as const;

const SQL_TYPE_COMPLETIONS = SQL_TYPE_NAMES.map(typeEntry);

const EMPTY_COMPLETION_DATA: SqlCompletionData = {
  namespaces: [],
  tablesByNamespace: {},
  columnsByTable: {},
  keywords: SQL_KEYWORDS,
  functions: SQL_FUNCTION_COMPLETIONS,
  snippets: SQL_SYNTAX_COMPLETIONS,
  types: SQL_TYPE_COMPLETIONS,
  operators: SQL_OPERATOR_COMPLETIONS,
  procedures: [],
  procedureEntries: [],
};

function pushUnique(values: string[], value: string) {
  if (!values.some((item) => item.toLowerCase() === value.toLowerCase())) {
    values.push(value);
  }
}

function addTable(
  data: Pick<SqlCompletionData, "columnsByTable" | "namespaces" | "tablesByNamespace">,
  namespaceName: string,
  tableName: string,
  columns: readonly string[],
) {
  const namespaceKey = namespaceName.toLowerCase();
  const tableKey = tableName.toLowerCase();
  const qualifiedTable = `${namespaceKey}.${tableKey}`;

  pushUnique(data.namespaces, namespaceName);
  if (!data.tablesByNamespace[namespaceKey]) {
    data.tablesByNamespace[namespaceKey] = [];
  }
  pushUnique(data.tablesByNamespace[namespaceKey], tableName);

  if (!data.columnsByTable[qualifiedTable]) {
    data.columnsByTable[qualifiedTable] = [];
  }
  columns.forEach((column) => pushUnique(data.columnsByTable[qualifiedTable], column));
}

export function buildSqlCompletionData(
  schema: StudioNamespace[],
  procedures: ProcedureMetadata[] = [],
): SqlCompletionData {
  const data: SqlCompletionData = {
    ...EMPTY_COMPLETION_DATA,
    namespaces: [],
    tablesByNamespace: {},
    columnsByTable: {},
    procedures,
    procedureEntries: procedureCompletionEntries(procedures),
  };

  schema.forEach((namespace) => {
    namespace.tables.forEach((table) => {
      addTable(data, namespace.name, table.name, table.columns.map((column) => column.name));
    });
  });

  STATIC_SYSTEM_TABLES.forEach((table) => {
    addTable(data, table.namespace, table.name, table.columns);
  });

  return data;
}

export function resolveSqlContextualCompletions(
  data: Pick<SqlCompletionData, "columnsByTable" | "tablesByNamespace">,
  textUntilPosition: string,
  aliasToTable: Record<string, string>,
): SqlContextualCompletionMatch | null {
  const tableColumnMatch = /([a-zA-Z_][\w]*)\.([a-zA-Z_][\w]*)\.([a-zA-Z_][\w]*)?$/.exec(textUntilPosition);
  if (tableColumnMatch) {
    const namespaceKey = tableColumnMatch[1].toLowerCase();
    const tableKey = tableColumnMatch[2].toLowerCase();
    const columns = data.columnsByTable[`${namespaceKey}.${tableKey}`] ?? [];
    return {
      kind: "column",
      labels: columns,
      detail: `${namespaceKey}.${tableKey} column`,
      partial: tableColumnMatch[3]?.toLowerCase() ?? "",
    };
  }

  const identifierMatch = /([a-zA-Z_][\w]*)\.([a-zA-Z_][\w]*)?$/.exec(textUntilPosition);
  if (!identifierMatch) {
    return null;
  }

  const qualifierKey = identifierMatch[1].toLowerCase();
  const partial = identifierMatch[2]?.toLowerCase() ?? "";
  const resolvedTable = aliasToTable[qualifierKey];
  if (resolvedTable) {
    return {
      kind: "column",
      labels: data.columnsByTable[resolvedTable] ?? [],
      detail: `${qualifierKey} alias column`,
      partial,
    };
  }

  const namespaceTables = data.tablesByNamespace[qualifierKey] ?? [];
  if (namespaceTables.length > 0) {
    return {
      kind: "table",
      labels: namespaceTables,
      detail: `${qualifierKey} table`,
      partial,
    };
  }

  return null;
}