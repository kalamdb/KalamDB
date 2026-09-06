use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RowFilterError {
    #[error("Failed to parse WHERE clause: {0}")]
    Parse(String),

    #[error("Invalid WHERE clause syntax")]
    InvalidSyntax,

    #[error(
        "WHERE subqueries require a query execution context and are not supported by row-local \
         filters"
    )]
    UnsupportedSubquery,

    #[error("unsupported row filter expression: {0}")]
    UnsupportedExpression(String),

    #[error("unsupported row filter operator: {0}")]
    UnsupportedOperator(String),

    #[error("{0}")]
    InvalidOperation(String),
}

pub type RowFilterResult<T> = std::result::Result<T, RowFilterError>;
