//! General SQL query feature support for DataFusion-backed execution.
//!
//! KalamDB routes most SELECT statements directly to DataFusion 54, which provides
//! SQL lambda array functions when the session uses the DuckDB SQL dialect
//! (`datafusion.sql_parser.dialect = duckdb`). Subscription and live-query paths
//! intentionally restrict the SQL surface and remain narrower.

/// DataFusion 54 query features supported in general SQL execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeneralQueryFeature {
    /// Lambda expressions such as `x -> x * 10` with `array_transform`, `array_filter`,
    /// and `array_any_match`.
    LambdaArrayFunctions,
}

impl GeneralQueryFeature {
    pub const ALL: &'static [Self] = &[Self::LambdaArrayFunctions];

    pub fn name(self) -> &'static str {
        match self {
            Self::LambdaArrayFunctions => "lambda_array_functions",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::LambdaArrayFunctions => {
                "SQL lambda expressions with array_transform, array_filter, and array_any_match"
            },
        }
    }
}

/// Returns `true` when the feature is available in general SQL execution.
pub fn supports_general_query_feature(feature: GeneralQueryFeature) -> bool {
    matches!(feature, GeneralQueryFeature::LambdaArrayFunctions)
}

/// Returns `true` when the feature is available in subscription/live-query SQL.
pub fn supports_subscription_query_feature(feature: GeneralQueryFeature) -> bool {
    !supports_general_query_feature(feature)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_queries_support_datafusion54_features() {
        for feature in GeneralQueryFeature::ALL {
            assert!(supports_general_query_feature(*feature));
        }
    }

    #[test]
    fn subscription_queries_keep_narrow_surface() {
        for feature in GeneralQueryFeature::ALL {
            assert!(!supports_subscription_query_feature(*feature));
        }
    }
}
