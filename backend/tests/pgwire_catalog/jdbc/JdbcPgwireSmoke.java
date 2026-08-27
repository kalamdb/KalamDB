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

    private JdbcPgwireSmoke() {}
}
