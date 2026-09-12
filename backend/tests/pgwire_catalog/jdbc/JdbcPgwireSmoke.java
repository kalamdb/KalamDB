import com.zaxxer.hikari.HikariConfig;
import com.zaxxer.hikari.HikariDataSource;
import java.sql.Connection;
import java.sql.DatabaseMetaData;
import java.sql.DriverManager;
import java.sql.PreparedStatement;
import java.sql.ResultSet;
import java.sql.Statement;
import java.util.Properties;

/**
 * PostgreSQL JDBC + HikariCP smoke against KalamDB pgwire.
 *
 * Args: jdbcUrl user password namespace table
 */
public final class JdbcPgwireSmoke {
    public static void main(String[] args) throws Exception {
        if (args.length != 5) {
            System.err.println("usage: JdbcPgwireSmoke jdbcUrl user password namespace table");
            System.exit(2);
        }
        String jdbcUrl = args[0];
        String user = args[1];
        String password = args[2];
        String namespace = args[3];
        String table = args[4];

        Class.forName("org.postgresql.Driver");
        driverManagerConnect(jdbcUrl, user, password);
        hikariPool(jdbcUrl, user, password, namespace, table);
        System.out.println("jdbc_pgwire_smoke_ok");
    }

    private static void driverManagerConnect(String jdbcUrl, String user, String password)
            throws Exception {
        Properties props = new Properties();
        props.setProperty("user", user);
        props.setProperty("password", password);
        props.setProperty("sslmode", "disable");
        props.setProperty("loginTimeout", "10");
        try (Connection conn = DriverManager.getConnection(jdbcUrl, props)) {
            assertIsolation(conn);
            System.out.println("driver_manager_ok");
        }
    }

    private static void hikariPool(
            String jdbcUrl, String user, String password, String namespace, String table)
            throws Exception {
        HikariConfig config = new HikariConfig();
        config.setJdbcUrl(jdbcUrl);
        config.setUsername(user);
        config.setPassword(password);
        config.setMaximumPoolSize(2);
        config.setMinimumIdle(0);
        config.setConnectionTimeout(15_000);
        config.setInitializationFailTimeout(15_000);
        config.addDataSourceProperty("sslmode", "disable");
        config.addDataSourceProperty("loginTimeout", "10");

        try (HikariDataSource ds = new HikariDataSource(config);
                Connection conn = ds.getConnection()) {
            System.out.println("hikari_pool_ok");
            assertIsolation(conn);
            if (!conn.getAutoCommit()) {
                throw new IllegalStateException("expected autocommit true");
            }
            System.out.println("autocommit_ok");

            try (Statement st = conn.createStatement();
                    ResultSet rs = st.executeQuery("SELECT 1")) {
                if (!rs.next() || rs.getInt(1) != 1) {
                    throw new IllegalStateException("SELECT 1 failed");
                }
            }
            System.out.println("select_1_ok");

            try (PreparedStatement ps = conn.prepareStatement("SELECT 1 AS n")) {
                try (ResultSet rs = ps.executeQuery()) {
                    if (!rs.next() || rs.getInt(1) != 1) {
                        throw new IllegalStateException("prepared SELECT 1 failed");
                    }
                }
            }
            System.out.println("prepared_select_ok");

            try (PreparedStatement ps = conn.prepareStatement("SELECT 1 AS n WHERE 1 = ?")) {
                ps.setInt(1, 1);
                try (ResultSet rs = ps.executeQuery()) {
                    if (!rs.next() || rs.getInt(1) != 1) {
                        throw new IllegalStateException("parameterized prepared query failed");
                    }
                }
            }
            System.out.println("prepared_param_ok");

            try (Statement st = conn.createStatement();
                    ResultSet rs = st.executeQuery("SHOW TRANSACTION ISOLATION LEVEL")) {
                if (!rs.next()) {
                    throw new IllegalStateException("SHOW TRANSACTION ISOLATION LEVEL returned no row");
                }
                String level = rs.getString(1);
                if (level == null || !level.equalsIgnoreCase("read committed")) {
                    throw new IllegalStateException("unexpected isolation SHOW value: " + level);
                }
            }
            System.out.println("show_isolation_ok");

            try (Statement st = conn.createStatement();
                    ResultSet rs = st.executeQuery("SHOW search_path")) {
                if (!rs.next() || rs.getString(1) == null) {
                    throw new IllegalStateException("SHOW search_path returned no value");
                }
            }
            System.out.println("show_search_path_ok");

            String qualified = namespace + "." + table;
            try (Statement st = conn.createStatement()) {
                st.execute("CREATE NAMESPACE IF NOT EXISTS " + namespace);
                st.execute(
                        "CREATE TABLE IF NOT EXISTS "
                                + qualified
                                + " (id INT PRIMARY KEY, name TEXT)");
                st.execute("INSERT INTO " + qualified + " (id, name) VALUES (1, 'jdbc')");
                st.execute(
                        "CREATE OR REPLACE PROCEDURE "
                                + namespace
                                + ".ping() RETURNS TEXT");
            }
            try (PreparedStatement ps =
                    conn.prepareStatement("SELECT name FROM " + qualified + " WHERE id = ?")) {
                ps.setInt(1, 1);
                try (ResultSet rs = ps.executeQuery()) {
                    if (!rs.next() || !"jdbc".equals(rs.getString(1))) {
                        throw new IllegalStateException("DML round-trip failed");
                    }
                }
            }
            System.out.println("dml_roundtrip_ok");

            DatabaseMetaData meta = conn.getMetaData();
            String product = meta.getDatabaseProductName();
            if (product == null || product.isEmpty()) {
                throw new IllegalStateException("DatabaseMetaData.getDatabaseProductName was empty");
            }
            System.out.println("database_product=" + product);

            try (PreparedStatement ps =
                    conn.prepareStatement(
                            "SELECT table_name FROM information_schema.tables "
                                    + "WHERE table_schema = ? AND table_name = ?")) {
                ps.setString(1, namespace);
                ps.setString(2, table);
                try (ResultSet rs = ps.executeQuery()) {
                    if (!rs.next() || !table.equals(rs.getString(1))) {
                        throw new IllegalStateException(
                                "information_schema.tables did not find " + qualified);
                    }
                }
            }
            System.out.println("information_schema_tables_ok");

            int maxName = meta.getMaxColumnNameLength();
            if (maxName <= 0) {
                throw new IllegalStateException("getMaxColumnNameLength returned " + maxName);
            }
            System.out.println("max_column_name_length=" + maxName);

            boolean foundCatalog = false;
            try (ResultSet rs = meta.getCatalogs()) {
                while (rs.next()) {
                    if ("kalam".equals(rs.getString("TABLE_CAT"))) {
                        foundCatalog = true;
                    }
                }
            }
            if (!foundCatalog) {
                throw new IllegalStateException("getCatalogs did not include kalam");
            }
            System.out.println("get_catalogs_ok");

            boolean foundSchema = false;
            try (ResultSet rs = meta.getSchemas(null, namespace)) {
                while (rs.next()) {
                    if (namespace.equals(rs.getString("TABLE_SCHEM"))) {
                        foundSchema = true;
                    }
                }
            }
            if (!foundSchema) {
                throw new IllegalStateException("getSchemas did not include " + namespace);
            }
            System.out.println("get_schemas_ok");

            boolean foundType = false;
            try (ResultSet rs = meta.getTableTypes()) {
                while (rs.next()) {
                    if ("TABLE".equals(rs.getString("TABLE_TYPE"))) {
                        foundType = true;
                    }
                }
            }
            if (!foundType) {
                throw new IllegalStateException("getTableTypes did not include TABLE");
            }
            System.out.println("get_table_types_ok");

            boolean foundTable = false;
            try (ResultSet rs = meta.getTables(null, namespace, table, new String[] {"TABLE"})) {
                while (rs.next()) {
                    if (table.equals(rs.getString("TABLE_NAME"))
                            && namespace.equals(rs.getString("TABLE_SCHEM"))) {
                        foundTable = true;
                    }
                }
            }
            if (!foundTable) {
                throw new IllegalStateException("getTables did not find " + qualified);
            }
            System.out.println("get_tables_ok");

            boolean foundId = false;
            boolean foundName = false;
            try (ResultSet rs = meta.getColumns(null, namespace, table, null)) {
                while (rs.next()) {
                    String column = rs.getString("COLUMN_NAME");
                    if ("id".equals(column)) {
                        foundId = true;
                    } else if ("name".equals(column)) {
                        foundName = true;
                    }
                }
            }
            if (!foundId || !foundName) {
                throw new IllegalStateException(
                        "getColumns missing id/name for " + qualified);
            }
            System.out.println("get_columns_ok");

            boolean foundPk = false;
            try (ResultSet rs = meta.getPrimaryKeys(null, namespace, table)) {
                while (rs.next()) {
                    if ("id".equals(rs.getString("COLUMN_NAME"))) {
                        foundPk = true;
                    }
                }
            }
            if (!foundPk) {
                throw new IllegalStateException("getPrimaryKeys did not find id for " + qualified);
            }
            System.out.println("get_primary_keys_ok");

            boolean foundIndex = false;
            try (ResultSet rs = meta.getIndexInfo(null, namespace, table, false, true)) {
                while (rs.next()) {
                    if ("id".equals(rs.getString("COLUMN_NAME"))) {
                        foundIndex = true;
                    }
                }
            }
            if (!foundIndex) {
                throw new IllegalStateException("getIndexInfo did not find id for " + qualified);
            }
            System.out.println("get_index_info_ok");

            try (ResultSet rs = meta.getImportedKeys(null, namespace, table)) {
                while (rs.next()) {
                    // no FKs expected; the call must succeed for GUI browsers
                }
            }
            System.out.println("get_imported_keys_ok");

            try (ResultSet rs = meta.getExportedKeys(null, namespace, table)) {
                while (rs.next()) {
                    // no FKs expected; the call must succeed for GUI browsers
                }
            }
            System.out.println("get_exported_keys_ok");

            boolean foundProcedure = false;
            try (ResultSet rs = meta.getProcedures(null, namespace, "ping")) {
                while (rs.next()) {
                    if ("ping".equals(rs.getString("PROCEDURE_NAME"))) {
                        foundProcedure = true;
                    }
                }
            }
            if (!foundProcedure) {
                throw new IllegalStateException("getProcedures did not find ping in " + namespace);
            }
            System.out.println("get_procedures_ok");

            try (ResultSet rs = meta.getProcedureColumns(null, namespace, "ping", null)) {
                while (rs.next()) {
                    // argument rows are optional; the call must succeed
                }
            }
            System.out.println("get_procedure_columns_ok");

            try (ResultSet rs = meta.getFunctions(null, namespace, "%")) {
                while (rs.next()) {
                    // Kalam CALL-able procedures use prokind='p'
                }
            }
            System.out.println("get_functions_ok");

            try (ResultSet rs = meta.getBestRowIdentifier(
                    null, namespace, table, DatabaseMetaData.bestRowTemporary, true)) {
                boolean foundBestRowPk = false;
                while (rs.next()) {
                    if ("id".equals(rs.getString("COLUMN_NAME"))) {
                        foundBestRowPk = true;
                    }
                }
                if (!foundBestRowPk) {
                    throw new IllegalStateException(
                            "getBestRowIdentifier missed PK id for " + qualified);
                }
            }
            System.out.println("get_best_row_identifier_ok");

            try (ResultSet rs = meta.getTablePrivileges(null, namespace, table)) {
                while (rs.next()) {
                    // empty privileges are fine
                }
            }
            System.out.println("get_table_privileges_ok");

            try (ResultSet rs = meta.getColumnPrivileges(null, namespace, table, "%")) {
                while (rs.next()) {
                    // empty privileges are fine
                }
            }
            System.out.println("get_column_privileges_ok");

            try (ResultSet rs = meta.getUDTs(null, namespace, "%", null)) {
                while (rs.next()) {
                    // no DISTINCT/STRUCT types expected
                }
            }
            System.out.println("get_udts_ok");

            try (PreparedStatement ps = conn.prepareStatement(TABULARIS_COLUMNS_SQL)) {
                ps.setString(1, namespace);
                ps.setString(2, table);
                try (ResultSet rs = ps.executeQuery()) {
                    boolean foundTabularisId = false;
                    boolean foundTabularisPk = false;
                    while (rs.next()) {
                        if ("id".equals(rs.getString("column_name"))) {
                            foundTabularisId = true;
                            foundTabularisPk = rs.getBoolean("is_pk");
                        }
                    }
                    if (!foundTabularisId || !foundTabularisPk) {
                        throw new IllegalStateException(
                                "Tabularis column probe missed PK id for " + qualified);
                    }
                }
            }
            System.out.println("tabularis_columns_ok");

            try (PreparedStatement ps = conn.prepareStatement(TABULARIS_INDEXES_SQL)) {
                ps.setString(1, namespace);
                ps.setString(2, table);
                try (ResultSet rs = ps.executeQuery()) {
                    boolean found = false;
                    while (rs.next()) {
                        if ("id".equals(rs.getString("column_name"))) {
                            found = true;
                        }
                    }
                    if (!found) {
                        throw new IllegalStateException(
                                "Tabularis index probe missed id for " + qualified);
                    }
                }
            }
            System.out.println("tabularis_indexes_ok");

            try (PreparedStatement ps = conn.prepareStatement(TABULARIS_FKEYS_SQL)) {
                ps.setString(1, namespace);
                ps.setString(2, table);
                try (ResultSet rs = ps.executeQuery()) {
                    while (rs.next()) {
                        // no FKs expected
                    }
                }
            }
            System.out.println("tabularis_fkeys_ok");

            try (PreparedStatement ps = conn.prepareStatement(TABULARIS_TRIGGERS_SQL)) {
                ps.setString(1, namespace);
                try (ResultSet rs = ps.executeQuery()) {
                    while (rs.next()) {
                        // no triggers expected
                    }
                }
            }
            System.out.println("tabularis_triggers_ok");

            try (PreparedStatement ps = conn.prepareStatement(TABULARIS_ROUTINES_SQL)) {
                ps.setString(1, namespace);
                try (ResultSet rs = ps.executeQuery()) {
                    boolean found = false;
                    while (rs.next()) {
                        if ("ping".equals(rs.getString("proname"))) {
                            found = true;
                        }
                    }
                    if (!found) {
                        throw new IllegalStateException(
                                "Tabularis routines probe missed ping in " + namespace);
                    }
                }
            }
            System.out.println("tabularis_routines_ok");

            try (PreparedStatement ps =
                    conn.prepareStatement(
                            "SELECT routine_name, routine_type FROM information_schema.routines "
                                    + "WHERE routine_schema = ? AND routine_name = ?")) {
                ps.setString(1, namespace);
                ps.setString(2, "ping");
                try (ResultSet rs = ps.executeQuery()) {
                    if (!rs.next() || !"PROCEDURE".equals(rs.getString("routine_type"))) {
                        throw new IllegalStateException(
                                "information_schema.routines missed ping in " + namespace);
                    }
                }
            }
            System.out.println("information_schema_routines_ok");

            try (PreparedStatement ps =
                    conn.prepareStatement(
                            "SELECT p.parameter_name FROM information_schema.parameters p "
                                    + "JOIN information_schema.routines r "
                                    + "ON p.specific_name = r.specific_name "
                                    + "WHERE r.routine_schema = ? AND r.routine_name = ?")) {
                ps.setString(1, namespace);
                ps.setString(2, "ping");
                try (ResultSet rs = ps.executeQuery()) {
                    while (rs.next()) {
                        // ping() has no arguments; the join must still succeed
                    }
                }
            }
            System.out.println("information_schema_parameters_ok");

            try (PreparedStatement ps = conn.prepareStatement(TABULARIS_ROUTINE_DEFINITION_SQL)) {
                ps.setString(1, namespace);
                ps.setString(2, "ping");
                try (ResultSet rs = ps.executeQuery()) {
                    if (!rs.next()) {
                        throw new IllegalStateException(
                                "Tabularis routine definition missed ping in " + namespace);
                    }
                    String definition = rs.getString("definition");
                    if (definition == null || !definition.toLowerCase().contains("ping")) {
                        throw new IllegalStateException(
                                "pg_get_functiondef did not return ping: " + definition);
                    }
                }
            }
            System.out.println("tabularis_routine_definition_ok");

            try (Statement st = conn.createStatement();
                    ResultSet rs = st.executeQuery("SELECT * FROM " + qualified)) {
                if (!rs.next()) {
                    throw new IllegalStateException("SELECT * returned no rows for " + qualified);
                }
                if (!"jdbc".equals(rs.getString("name"))) {
                    throw new IllegalStateException("SELECT * did not return inserted row");
                }
            }
            System.out.println("select_star_ok");

            String keywords = meta.getSQLKeywords();
            if (keywords == null) {
                throw new IllegalStateException("getSQLKeywords returned null");
            }
            System.out.println("get_sql_keywords_ok");

            String uuidTable = table + "_uuid";
            String uuidQualified = namespace + "." + uuidTable;
            try (Statement st = conn.createStatement()) {
                st.execute(
                        "CREATE TABLE IF NOT EXISTS "
                                + uuidQualified
                                + " (id UUID PRIMARY KEY, name TEXT)");
            }
            boolean foundUuidId = false;
            boolean foundUuidName = false;
            try (ResultSet rs = meta.getColumns(null, namespace, uuidTable, null)) {
                while (rs.next()) {
                    String column = rs.getString("COLUMN_NAME");
                    if ("id".equals(column)) {
                        foundUuidId = true;
                    } else if ("name".equals(column)) {
                        foundUuidName = true;
                    }
                }
            }
            if (!foundUuidId || !foundUuidName) {
                throw new IllegalStateException(
                        "getColumns missing id/name for " + uuidQualified);
            }
            System.out.println("get_uuid_columns_ok");
        }
    }

    private static void assertIsolation(Connection conn) throws Exception {
        int isolation = conn.getTransactionIsolation();
        if (isolation != Connection.TRANSACTION_READ_COMMITTED) {
            throw new IllegalStateException("unexpected isolation constant: " + isolation);
        }
        System.out.println("isolation=" + isolation);
    }

    private static final String TABULARIS_COLUMNS_SQL =
            "SELECT c.column_name::text, CASE WHEN c.data_type = 'USER-DEFINED' THEN "
                    + "c.udt_name::text ELSE c.data_type::text END AS data_type, "
                    + "c.is_nullable::text, c.column_default::text, c.is_identity::text, "
                    + "c.character_maximum_length, "
                    + "(SELECT string_agg('''' || replace(e.enumlabel, '''', '''''') || '''', ',' "
                    + "ORDER BY e.enumsortorder) FROM pg_enum e JOIN pg_type t ON t.oid = e.enumtypid "
                    + "JOIN pg_namespace tn ON tn.oid = t.typnamespace WHERE t.typname = c.udt_name "
                    + "AND tn.nspname = c.udt_schema) AS enum_values, EXISTS ( SELECT 1 FROM "
                    + "pg_constraint pk_con JOIN pg_class pk_table ON pk_table.oid = pk_con.conrelid "
                    + "JOIN pg_namespace pk_schema ON pk_schema.oid = pk_table.relnamespace JOIN "
                    + "unnest(pk_con.conkey) AS pk_col(attnum) ON true JOIN pg_attribute pk_att ON "
                    + "pk_att.attrelid = pk_table.oid AND pk_att.attnum = pk_col.attnum AND NOT "
                    + "pk_att.attisdropped WHERE pk_con.contype = 'p' AND pk_schema.nspname = "
                    + "c.table_schema AND pk_table.relname = c.table_name AND pk_att.attname = "
                    + "c.column_name ) AS is_pk FROM information_schema.columns c WHERE "
                    + "c.table_schema = ? AND c.table_name = ? ORDER BY c.ordinal_position";

    private static final String TABULARIS_INDEXES_SQL =
            "SELECT i.relname AS index_name, COALESCE(a.attname::text, "
                    + "pg_get_indexdef(ix.indexrelid, k.n::int, true)) AS column_name, "
                    + "ix.indisunique AS is_unique, ix.indisprimary AS is_primary, k.n::int AS "
                    + "seq_in_index, (k.attnum = 0) AS is_expression FROM pg_class t JOIN "
                    + "pg_namespace n ON t.relnamespace = n.oid JOIN pg_index ix ON t.oid = "
                    + "ix.indrelid JOIN pg_class i ON i.oid = ix.indexrelid CROSS JOIN LATERAL "
                    + "unnest(string_to_array(ix.indkey::text, ' ')::int2[]) WITH ORDINALITY AS "
                    + "k(attnum, n) LEFT JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = "
                    + "k.attnum AND k.attnum <> 0 WHERE t.relkind IN ('r', 'm') AND n.nspname = ? "
                    + "AND t.relname = ? ORDER BY i.relname, k.n";

    private static final String TABULARIS_FKEYS_SQL =
            "SELECT con.conname::text AS constraint_name, src_att.attname::text AS column_name, "
                    + "ref_nsp.nspname::text AS foreign_schema_name, ref_cl.relname::text AS "
                    + "foreign_table_name, ref_att.attname::text AS foreign_column_name FROM "
                    + "pg_constraint con JOIN pg_class src_cl ON src_cl.oid = con.conrelid JOIN "
                    + "pg_namespace src_nsp ON src_nsp.oid = src_cl.relnamespace JOIN pg_class "
                    + "ref_cl ON ref_cl.oid = con.confrelid JOIN pg_namespace ref_nsp ON "
                    + "ref_nsp.oid = ref_cl.relnamespace JOIN unnest(con.conkey, con.confkey) AS "
                    + "cols(src_attnum, ref_attnum) ON true JOIN pg_attribute src_att ON "
                    + "src_att.attrelid = src_cl.oid AND src_att.attnum = cols.src_attnum AND NOT "
                    + "src_att.attisdropped JOIN pg_attribute ref_att ON ref_att.attrelid = "
                    + "ref_cl.oid AND ref_att.attnum = cols.ref_attnum AND NOT "
                    + "ref_att.attisdropped WHERE con.contype = 'f' AND con.conparentid = 0 AND "
                    + "src_nsp.nspname = ? AND src_cl.relname = ? ORDER BY con.conname, "
                    + "cols.src_attnum";

    private static final String TABULARIS_TRIGGERS_SQL =
            "SELECT t.trigger_name AS name, t.event_object_table AS table_name, "
                    + "string_agg(t.event_manipulation, ' OR ' ORDER BY t.event_manipulation) AS "
                    + "event, t.action_timing AS timing FROM information_schema.triggers t WHERE "
                    + "t.trigger_schema = ? GROUP BY t.trigger_name, t.event_object_table, "
                    + "t.action_timing ORDER BY t.trigger_name";

    private static final String TABULARIS_ROUTINES_SQL =
            "SELECT proname, prokind FROM pg_proc WHERE pronamespace = (SELECT oid FROM "
                    + "pg_namespace WHERE nspname = ?) AND prokind IN ('f', 'p') ORDER BY proname";

    private static final String TABULARIS_ROUTINE_DEFINITION_SQL =
            "SELECT pg_get_functiondef(p.oid) as definition FROM pg_proc p JOIN pg_namespace n "
                    + "ON p.pronamespace = n.oid WHERE n.nspname = ? AND p.proname = ? LIMIT 1";

    private JdbcPgwireSmoke() {}
}
