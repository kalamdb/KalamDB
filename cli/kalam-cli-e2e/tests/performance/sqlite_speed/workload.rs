//! Statement mix for the SQLite speed suite published by PGlite.
//!
//! The scripts follow `packages/benchmark/src/benchmarkN.sql` in
//! <https://github.com/electric-sql/pglite>: the same statement counts, the same
//! filters, and English number-names in the text column. Values are a
//! deterministic 0..99999 sequence with that spelling, so `LIKE` selectivity
//! matches the published suite.

/// SQL text cap enforced by the server, minus room for `USE SCHEMA` / `BEGIN` / `COMMIT`.
pub const SQL_BUDGET: usize = 4 * 1024 * 1024 - 128;

/// Published native-baseline seconds from <https://pglite.dev/benchmarks>.
/// Columns are SQLite in memory, SQLite on disk, and Postgres.
pub struct PublishedPhase {
    pub id:                 &'static str,
    pub name:               &'static str,
    pub sqlite_memory_secs: f64,
    pub sqlite_disk_secs:   f64,
    pub postgres_secs:      f64,
}

pub const PUBLISHED: [PublishedPhase; 16] = [
    PublishedPhase {
        id:                 "1",
        name:               "1000 INSERTs",
        sqlite_memory_secs: 0.002,
        sqlite_disk_secs:   0.288,
        postgres_secs:      0.007,
    },
    PublishedPhase {
        id:                 "2",
        name:               "25000 INSERTs in a transaction",
        sqlite_memory_secs: 0.022,
        sqlite_disk_secs:   0.019,
        postgres_secs:      0.114,
    },
    PublishedPhase {
        id:                 "3",
        name:               "25000 INSERTs into an indexed table",
        sqlite_memory_secs: 0.035,
        sqlite_disk_secs:   0.04,
        postgres_secs:      0.383,
    },
    PublishedPhase {
        id:                 "4",
        name:               "100 SELECTs without an index",
        sqlite_memory_secs: 0.076,
        sqlite_disk_secs:   0.078,
        postgres_secs:      0.094,
    },
    PublishedPhase {
        id:                 "5",
        name:               "100 SELECTs on a string comparison",
        sqlite_memory_secs: 0.268,
        sqlite_disk_secs:   0.429,
        postgres_secs:      0.259,
    },
    PublishedPhase {
        id:                 "6",
        name:               "Creating an index",
        sqlite_memory_secs: 0.007,
        sqlite_disk_secs:   0.011,
        postgres_secs:      0.01,
    },
    PublishedPhase {
        id:                 "7",
        name:               "5000 SELECTs with an index",
        sqlite_memory_secs: 0.01,
        sqlite_disk_secs:   0.01,
        postgres_secs:      0.078,
    },
    PublishedPhase {
        id:                 "8",
        name:               "1000 UPDATEs without an index",
        sqlite_memory_secs: 0.018,
        sqlite_disk_secs:   0.021,
        postgres_secs:      0.047,
    },
    PublishedPhase {
        id:                 "9",
        name:               "25000 UPDATEs with an index",
        sqlite_memory_secs: 0.047,
        sqlite_disk_secs:   0.056,
        postgres_secs:      0.307,
    },
    PublishedPhase {
        id:                 "10",
        name:               "25000 text UPDATEs with an index",
        sqlite_memory_secs: 0.032,
        sqlite_disk_secs:   0.041,
        postgres_secs:      0.416,
    },
    PublishedPhase {
        id:                 "11",
        name:               "INSERTs from a SELECT",
        sqlite_memory_secs: 0.022,
        sqlite_disk_secs:   0.027,
        postgres_secs:      0.072,
    },
    PublishedPhase {
        id:                 "12",
        name:               "DELETE without an index",
        sqlite_memory_secs: 0.01,
        sqlite_disk_secs:   0.023,
        postgres_secs:      0.007,
    },
    PublishedPhase {
        id:                 "13",
        name:               "DELETE with an index",
        sqlite_memory_secs: 0.017,
        sqlite_disk_secs:   0.021,
        postgres_secs:      0.019,
    },
    PublishedPhase {
        id:                 "14",
        name:               "A big INSERT after a big DELETE",
        sqlite_memory_secs: 0.017,
        sqlite_disk_secs:   0.021,
        postgres_secs:      0.048,
    },
    PublishedPhase {
        id:                 "15",
        name:               "A big DELETE followed by many small INSERTs",
        sqlite_memory_secs: 0.008,
        sqlite_disk_secs:   0.01,
        postgres_secs:      0.067,
    },
    PublishedPhase {
        id:                 "16",
        name:               "DROP TABLE",
        sqlite_memory_secs: 0.001,
        sqlite_disk_secs:   0.003,
        postgres_secs:      0.004,
    },
];

const ONES: [&str; 20] = [
    "zero",
    "one",
    "two",
    "three",
    "four",
    "five",
    "six",
    "seven",
    "eight",
    "nine",
    "ten",
    "eleven",
    "twelve",
    "thirteen",
    "fourteen",
    "fifteen",
    "sixteen",
    "seventeen",
    "eighteen",
    "nineteen",
];

const TENS: [&str; 10] = [
    "", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub a: i64,
    pub b: i64,
    pub c: String,
}

pub struct SpeedRng {
    state: u64,
}

impl SpeedRng {
    pub fn new(seed: u64) -> Self {
        Self { state: seed | 1 }
    }

    pub fn next_value(&mut self) -> i64 {
        self.state = self.state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.state % 100_000) as i64
    }
}

pub fn number_name(n: i64) -> String {
    assert!((0..100_000).contains(&n), "number name is defined for 0..99999");
    if n < 1000 {
        return spell_under_1000(n);
    }
    let thousands = n / 1000;
    let rest = n % 1000;
    let mut name = format!("{} thousand", spell_under_1000(thousands));
    if rest != 0 {
        name.push(' ');
        name.push_str(&spell_under_1000(rest));
    }
    name
}

/// The 100 `LIKE` needles from benchmark 5, in order.
pub fn like_needles() -> Vec<String> {
    let mut needles = Vec::with_capacity(100);
    for n in 1..20 {
        needles.push(spell_under_100(n));
    }
    for n in 20..100 {
        needles.push(spell_under_100(n));
    }
    needles.push("one hundred".to_string());
    needles
}

pub fn gen_rows(seed: u64, count: usize) -> Vec<Row> {
    let mut rng = SpeedRng::new(seed);
    let mut rows = Vec::with_capacity(count);
    for a in 1..=count {
        let b = rng.next_value();
        let c = number_name(b);
        rows.push(Row { a: a as i64, b, c });
    }
    rows
}

pub fn create_table_sql(table: &str) -> String {
    format!("CREATE TABLE {table}(a INTEGER, b INTEGER, c VARCHAR(100))")
}

pub fn insert_statement(table: &str, row: &Row) -> String {
    format!("INSERT INTO {table} VALUES({}, {}, '{}')", row.a, row.b, row.c)
}

pub fn insert_statements(table: &str, rows: &[Row]) -> Vec<String> {
    rows.iter().map(|row| insert_statement(table, row)).collect()
}

/// Split statements only when one request would exceed the server SQL limit.
///
/// Transactional scripts wrap each chunk in `BEGIN` / `COMMIT`. A published
/// phase fits in one request, so it stays one transaction.
pub fn chunk_statements(transactional: bool, statements: &[String]) -> Vec<String> {
    chunk_statements_capped(transactional, statements, usize::MAX)
}

/// Like [`chunk_statements`], but also closes a chunk after `max_statements`.
pub fn chunk_statements_capped(
    transactional: bool,
    statements: &[String],
    max_statements: usize,
) -> Vec<String> {
    if statements.is_empty() {
        return Vec::new();
    }

    let header = if transactional { "BEGIN;\n" } else { "" };
    let footer = if transactional { "COMMIT;\n" } else { "" };
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut open = false;
    let mut count = 0usize;

    for statement in statements {
        let line_len = statement.len() + 2;
        let over_bytes = current.len() + line_len + footer.len() > SQL_BUDGET;
        if open && (count >= max_statements || over_bytes) {
            current.push_str(footer);
            chunks.push(std::mem::take(&mut current));
            open = false;
            count = 0;
        }
        if !open {
            current.push_str(header);
            open = true;
        }
        current.push_str(statement);
        current.push_str(";\n");
        count += 1;
    }

    if open {
        current.push_str(footer);
        chunks.push(current);
    }

    for chunk in &chunks {
        assert!(
            chunk.len() <= SQL_BUDGET + 16,
            "SQL chunk is {} bytes, over the server limit",
            chunk.len()
        );
    }
    chunks
}

fn spell_under_100(n: i64) -> String {
    if n < 20 {
        return ONES[n as usize].to_string();
    }
    let tens = TENS[(n / 10) as usize];
    let ones = (n % 10) as usize;
    if ones == 0 {
        tens.to_string()
    } else {
        format!("{tens}-{}", ONES[ones])
    }
}

fn spell_under_1000(n: i64) -> String {
    let hundreds = n / 100;
    let rest = n % 100;
    if hundreds == 0 {
        return if rest == 0 {
            "zero".to_string()
        } else {
            spell_under_100(rest)
        };
    }
    let mut name = format!("{} hundred", ONES[hundreds as usize]);
    if rest != 0 {
        name.push(' ');
        name.push_str(&spell_under_100(rest));
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_names_match_the_published_spellings() {
        assert_eq!(number_name(19147), "nineteen thousand one hundred forty-seven");
        assert_eq!(number_name(26008), "twenty-six thousand eight");
        assert_eq!(number_name(46582), "forty-six thousand five hundred eighty-two");
        assert_eq!(number_name(75112), "seventy-five thousand one hundred twelve");
        assert_eq!(number_name(27011), "twenty-seven thousand eleven");
        assert_eq!(number_name(5904), "five thousand nine hundred four");
        assert_eq!(number_name(69001), "sixty-nine thousand one");
        assert_eq!(number_name(0), "zero");
        assert_eq!(number_name(100), "one hundred");
    }

    #[test]
    fn like_needles_follow_benchmark_5() {
        let needles = like_needles();
        assert_eq!(needles.len(), 100);
        assert_eq!(needles[0], "one");
        assert_eq!(needles[19], "twenty");
        assert_eq!(needles[99], "one hundred");
        assert!(needles.iter().any(|needle| needle == "fifty"));
    }

    #[test]
    fn one_transaction_holds_every_insert() {
        let rows = gen_rows(2, 25_000);
        let statements = insert_statements("t2", &rows);
        let chunks = chunk_statements(true, &statements);
        assert_eq!(chunks.len(), 1, "25000 inserts must stay one transaction");
        let chunk = &chunks[0];
        assert_eq!(chunk.matches("INSERT INTO t2 VALUES(").count(), 25_000);
        assert!(chunk.starts_with("BEGIN;\n"));
        assert!(chunk.ends_with("COMMIT;\n"));
        assert!(
            !chunk.contains("row_id") && !chunk.contains("TYPE"),
            "benchmark SQL must not add a surrogate key or a table type"
        );
    }
}
