//! `where` filters for `GET /query`: parsing the clause syntax and rendering each
//! clause as a SQL condition, with values checked against the column's type.
//!
//! | clause | condition |
//! |---|---|
//! | `col:value`, `col:eq:value` | `col = value` |
//! | `col:ne:value` | `col <> value` |
//! | `col:gt:value` (`gte`, `lt`, `lte`) | `col > value` (`>=`, `<`, `<=`) |
//! | `col:between:lo\|hi` | `col BETWEEN lo AND hi` |
//! | `col:in:a\|b`, `col:nin:a\|b` | `col IN (a, b)`, `col NOT IN (a, b)` |
//! | `col:isnull`, `col:notnull` | `col IS NULL`, `col IS NOT NULL` |
//!
//! A value that merely looks like an operator is matched with `eq`:
//! `col:eq:gt:5` is `col = 'gt:5'`.

use crate::db;
use crate::error::AppError;
use duckdb::Connection;
use std::sync::Mutex;

/// One parsed `where` clause.
#[derive(Debug, Clone, PartialEq)]
pub struct Filter {
    pub column: String,
    pub op: Op,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Eq(String),
    Ne(String),
    Gt(String),
    Gte(String),
    Lt(String),
    Lte(String),
    Between(String, String),
    In(Vec<String>),
    NotIn(Vec<String>),
    IsNull,
    NotNull,
}

/// Parse every `where` parameter (each a comma-separated list of clauses).
pub fn parse_where<'a>(params: impl IntoIterator<Item = &'a str>) -> Result<Vec<Filter>, AppError> {
    params
        .into_iter()
        .flat_map(|p| p.split(','))
        .filter(|c| !c.trim().is_empty())
        .map(parse_clause)
        .collect()
}

fn parse_clause(clause: &str) -> Result<Filter, AppError> {
    let (column, rest) = clause.split_once(':').ok_or_else(|| {
        AppError::BadRequest(format!(
            "Invalid where clause '{}'. Use column:value or column:op:value",
            clause
        ))
    })?;
    let column = column.trim().to_string();
    db::validate_column_name(&column)?;

    let op = match rest.split_once(':') {
        Some((op, value)) => match op {
            "eq" => Some(Op::Eq(value.to_string())),
            "ne" => Some(Op::Ne(value.to_string())),
            "gt" => Some(Op::Gt(value.to_string())),
            "gte" => Some(Op::Gte(value.to_string())),
            "lt" => Some(Op::Lt(value.to_string())),
            "lte" => Some(Op::Lte(value.to_string())),
            "between" => match value.split('|').collect::<Vec<_>>()[..] {
                [lo, hi] => Some(Op::Between(lo.to_string(), hi.to_string())),
                _ => {
                    return Err(AppError::BadRequest(format!(
                        "between needs exactly two values, lo|hi: '{}'",
                        clause
                    )))
                }
            },
            "in" => Some(Op::In(list(value))),
            "nin" => Some(Op::NotIn(list(value))),
            _ => None,
        },
        None => match rest {
            "isnull" => Some(Op::IsNull),
            "notnull" => Some(Op::NotNull),
            _ => None,
        },
    };
    // Anything else is the original form, column:value (the value may contain ':').
    let op = op.unwrap_or_else(|| Op::Eq(rest.to_string()));
    Ok(Filter { column, op })
}

fn list(value: &str) -> Vec<String> {
    value.split('|').map(str::to_string).collect()
}

/// A SQL condition built from a validated `where` clause. Only this module can
/// make one, so the query builder never takes raw SQL from its callers.
#[derive(Debug, Clone, PartialEq)]
pub struct Condition(String);

impl Condition {
    pub fn sql(&self) -> &str {
        &self.0
    }
}

/// Resolve each filter's column in `columns` (name, DuckDB type) and render it as
/// a [`Condition`]. Names match case-insensitively, as DuckDB's do. Date and time
/// values are checked with DuckDB's own `TRY_CAST` (a bound parameter, so the
/// value never reaches the SQL text) before the query runs.
pub fn to_conditions(
    filters: &[Filter],
    columns: &[(&str, &str)],
    db: &Mutex<Connection>,
) -> Result<Vec<Condition>, AppError> {
    filters
        .iter()
        .map(|f| {
            let (name, dtype) = columns
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(&f.column))
                .ok_or_else(|| AppError::ColumnNotFound(f.column.clone()))?;
            let (sql, casts) = render(f, name, dtype)?;
            for (value, ty) in casts {
                check_cast(db, f, &value, ty)?;
            }
            Ok(Condition(sql))
        })
        .collect()
}

fn check_cast(db: &Mutex<Connection>, f: &Filter, value: &str, ty: &str) -> Result<(), AppError> {
    let conn = db::lock_db(db)?;
    // `ty` is one of the fixed type keywords in `TEMPORAL`, never request text.
    let ok: bool = conn
        .query_row(
            &format!("SELECT TRY_CAST(? AS {}) IS NOT NULL", ty),
            [value],
            |row| row.get(0),
        )
        .map_err(|e| AppError::Internal(anyhow::anyhow!("cast check failed: {}", e)))?;
    if ok {
        Ok(())
    } else {
        Err(AppError::BadRequest(format!(
            "Invalid value '{}' for column '{}' ({})",
            value, f.column, ty
        )))
    }
}

/// Date and time types: compared as their type, with the literal cast by DuckDB.
const TEMPORAL: &[&str] = &[
    "DATE",
    "TIME",
    "TIMETZ",
    "TIME WITH TIME ZONE",
    "TIMESTAMP",
    "TIMESTAMPTZ",
    "TIMESTAMP WITH TIME ZONE",
    "TIMESTAMP_S",
    "TIMESTAMP_MS",
    "TIMESTAMP_NS",
    "INTERVAL",
];

/// How a column's values compare, from its DuckDB type.
#[derive(Clone, Copy)]
enum Kind {
    Integer,
    Float,
    Boolean,
    Temporal(&'static str),
    /// Everything else (VARCHAR, ENUM, UUID, nested types, ...): compared as text,
    /// as `where` always did.
    Text,
}

fn kind(dtype: &str) -> Kind {
    let upper = dtype.trim().to_ascii_uppercase();
    let base = upper.split('(').next().unwrap_or(&upper).trim();
    match base {
        "TINYINT" | "SMALLINT" | "INTEGER" | "BIGINT" | "HUGEINT" | "UTINYINT" | "USMALLINT"
        | "UINTEGER" | "UBIGINT" | "UHUGEINT" => Kind::Integer,
        "FLOAT" | "REAL" | "DOUBLE" | "DECIMAL" | "NUMERIC" => Kind::Float,
        "BOOLEAN" => Kind::Boolean,
        _ => match TEMPORAL.iter().find(|t| **t == base) {
            Some(t) => Kind::Temporal(t),
            None => Kind::Text,
        },
    }
}

/// A value DuckDB must be able to cast to a type: (value, type keyword).
type Cast = (String, &'static str);

/// The SQL for `filter` on column `name` of type `dtype`, plus the (value, type)
/// pairs DuckDB must be able to cast for it to be valid.
fn render(filter: &Filter, name: &str, dtype: &str) -> Result<(String, Vec<Cast>), AppError> {
    let kind = kind(dtype);
    let col = match kind {
        Kind::Text => format!("CAST(\"{}\" AS VARCHAR)", name),
        _ => format!("\"{}\"", name),
    };
    let mut casts = Vec::new();
    let mut lit = |v: &str| -> Result<String, AppError> {
        let t = v.trim();
        let invalid = || {
            AppError::BadRequest(format!(
                "Invalid value '{}' for column '{}' ({})",
                v, filter.column, dtype
            ))
        };
        match kind {
            Kind::Integer => t
                .parse::<i128>()
                .map(|n| n.to_string())
                .map_err(|_| invalid()),
            // The validated text, not the f64, so a decimal keeps its exact digits.
            Kind::Float => match t.parse::<f64>() {
                Ok(x) if x.is_finite() => Ok(t.to_string()),
                _ => Err(invalid()),
            },
            Kind::Boolean => match t.to_ascii_lowercase().as_str() {
                "true" => Ok("TRUE".to_string()),
                "false" => Ok("FALSE".to_string()),
                _ => Err(invalid()),
            },
            Kind::Temporal(ty) => {
                casts.push((t.to_string(), ty));
                Ok(format!("CAST({} AS {})", quote(t), ty))
            }
            Kind::Text => Ok(quote(v)),
        }
    };
    let sql = match &filter.op {
        Op::Eq(v) => format!("{} = {}", col, lit(v)?),
        Op::Ne(v) => format!("{} <> {}", col, lit(v)?),
        Op::Gt(v) => format!("{} > {}", col, lit(v)?),
        Op::Gte(v) => format!("{} >= {}", col, lit(v)?),
        Op::Lt(v) => format!("{} < {}", col, lit(v)?),
        Op::Lte(v) => format!("{} <= {}", col, lit(v)?),
        Op::Between(lo, hi) => {
            let (lo, hi) = (lit(lo)?, lit(hi)?);
            format!("{} BETWEEN {} AND {}", col, lo, hi)
        }
        Op::In(vs) | Op::NotIn(vs) => {
            let items = vs.iter().map(|v| lit(v)).collect::<Result<Vec<_>, _>>()?;
            let not = if matches!(filter.op, Op::NotIn(_)) {
                "NOT "
            } else {
                ""
            };
            format!("{} {}IN ({})", col, not, items.join(", "))
        }
        Op::IsNull => format!("{} IS NULL", col),
        Op::NotNull => format!("{} IS NOT NULL", col),
    };
    Ok((sql, casts))
}

fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(clause: &str) -> Filter {
        let mut f = parse_where([clause]).unwrap();
        assert_eq!(f.len(), 1);
        f.remove(0)
    }

    #[test]
    fn parses_every_operator() {
        assert_eq!(one("species:Bombus").op, Op::Eq("Bombus".into()));
        assert_eq!(one("count:eq:3").op, Op::Eq("3".into()));
        assert_eq!(one("count:ne:3").op, Op::Ne("3".into()));
        assert_eq!(one("count:gt:3").op, Op::Gt("3".into()));
        assert_eq!(one("count:gte:3").op, Op::Gte("3".into()));
        assert_eq!(one("count:lt:3").op, Op::Lt("3".into()));
        assert_eq!(one("count:lte:3").op, Op::Lte("3".into()));
        assert_eq!(
            one("count:between:2|5").op,
            Op::Between("2".into(), "5".into())
        );
        assert_eq!(
            one("species:in:A|B").op,
            Op::In(vec!["A".into(), "B".into()])
        );
        assert_eq!(one("species:nin:A").op, Op::NotIn(vec!["A".into()]));
        assert_eq!(one("observed_on:isnull").op, Op::IsNull);
        assert_eq!(one("observed_on:notnull").op, Op::NotNull);
    }

    #[test]
    fn keeps_the_original_column_value_form() {
        // A value with a colon that isn't an operator stays an equality.
        assert_eq!(one("time:12:30").op, Op::Eq("12:30".into()));
        // eq matches a value that looks like an operator.
        assert_eq!(one("note:eq:gt:5").op, Op::Eq("gt:5".into()));
        assert_eq!(one("note:eq:isnull").op, Op::Eq("isnull".into()));
    }

    #[test]
    fn splits_commas_and_joins_repeated_params() {
        let f = parse_where(["a:1,b:2", "c:3"]).unwrap();
        assert_eq!(
            f.iter().map(|f| f.column.as_str()).collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
    }

    #[test]
    fn rejects_malformed_clauses() {
        assert!(parse_where(["nocolon"]).is_err());
        assert!(parse_where(["count:between:5"]).is_err());
        assert!(parse_where(["bad;col:1"]).is_err());
    }

    fn sql(clause: &str, dtype: &str) -> String {
        render(&one(clause), "c", dtype).unwrap().0
    }

    #[test]
    fn renders_typed_literals() {
        assert_eq!(sql("c:gt:5", "BIGINT"), "\"c\" > 5");
        assert_eq!(
            sql("c:between:35.5|36", "DOUBLE"),
            "\"c\" BETWEEN 35.5 AND 36"
        );
        assert_eq!(sql("c:lt:1.10", "DECIMAL(18,3)"), "\"c\" < 1.10");
        assert_eq!(sql("c:eq:TRUE", "BOOLEAN"), "\"c\" = TRUE");
        assert_eq!(sql("c:isnull", "BIGINT"), "\"c\" IS NULL");
        assert_eq!(
            sql("c:gte:2024-03-17", "DATE"),
            "\"c\" >= CAST('2024-03-17' AS DATE)"
        );
    }

    #[test]
    fn compares_text_and_other_types_as_text() {
        assert_eq!(
            sql("c:in:O'Hara|B", "VARCHAR"),
            "CAST(\"c\" AS VARCHAR) IN ('O''Hara', 'B')"
        );
        assert_eq!(
            sql("c:nin:x", "ENUM('casual', 'research')"),
            "CAST(\"c\" AS VARCHAR) NOT IN ('x')"
        );
        assert_eq!(
            sql("c:eq:a", "STRUCT(\"weird name\" INTEGER)"),
            "CAST(\"c\" AS VARCHAR) = 'a'"
        );
    }

    #[test]
    fn asks_duckdb_to_check_temporal_values() {
        let (_, casts) = render(&one("c:between:2024-01-01|2024-02-01"), "c", "DATE").unwrap();
        assert_eq!(
            casts,
            [
                ("2024-01-01".to_string(), "DATE"),
                ("2024-02-01".to_string(), "DATE")
            ]
        );
        let (_, casts) = render(&one("c:gt:3"), "c", "BIGINT").unwrap();
        assert!(casts.is_empty());
    }

    #[test]
    fn rejects_values_of_the_wrong_type() {
        assert!(render(&one("c:gt:many"), "c", "BIGINT").is_err());
        assert!(render(&one("c:lt:inf"), "c", "DOUBLE").is_err());
        assert!(render(&one("c:eq:yes"), "c", "BOOLEAN").is_err());
        assert!(parse_where(["c:between:A|M|Z"]).is_err());
    }
}
