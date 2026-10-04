//! Rows read by column name, so that a column a browser version lacks, or
//! one it added, changes nothing: a missing column reads as NULL.

use std::collections::HashMap;

use sqlite::{Database, Row, Value};

/// A row with its table's column names.
pub(crate) struct Named<'t> {
    columns: &'t [String],
    pub(crate) rowid: i64,
    values: Vec<Value>,
}

impl Named<'_> {
    fn value(&self, column: &str) -> Option<&Value> {
        let at = self
            .columns
            .iter()
            .position(|name| name.eq_ignore_ascii_case(column))?;
        self.values.get(at)
    }

    /// The column's integer; `None` when it is NULL, not an integer, or
    /// not in the table.
    pub(crate) fn integer(&self, column: &str) -> Option<i64> {
        self.value(column).and_then(Value::as_integer)
    }

    /// The column's text; `None` when it is NULL, not text, or not in the
    /// table.
    pub(crate) fn text(&self, column: &str) -> Option<String> {
        self.value(column)
            .and_then(Value::as_text)
            .map(str::to_owned)
    }

    /// The column's integer as a flag: non-zero is true.
    pub(crate) fn flag(&self, column: &str) -> Option<bool> {
        self.integer(column).map(|value| value != 0)
    }

    /// A reference to another row: `0` and NULL mean none.
    pub(crate) fn reference(&self, column: &str) -> Option<i64> {
        self.integer(column).filter(|&id| id != 0)
    }
}

/// Whether `table` exists and declares `column`.
pub(crate) fn has_column(db: &Database<'_>, table: &str, column: &str) -> bool {
    db.table(table).is_some_and(|t| {
        t.column_names()
            .iter()
            .any(|name| name.eq_ignore_ascii_case(column))
    })
}

/// Every row of `table` in rowid order, converted; none when the table is
/// absent. Damage met on the way is added to `problems`.
pub(crate) fn read<T>(
    db: &Database<'_>,
    table: &str,
    problems: &mut Vec<String>,
    mut convert: impl FnMut(&Named<'_>) -> T,
) -> Vec<T> {
    let Some(columns) = db.table(table).map(|t| {
        t.column_names()
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>()
    }) else {
        return Vec::new();
    };
    let mut rows = match db.rows(table) {
        Ok(rows) => rows,
        Err(e) => {
            problems.push(format!("{table}: {e}"));
            return Vec::new();
        }
    };
    let converted = rows
        .by_ref()
        .map(|Row { rowid, values, .. }| {
            convert(&Named {
                columns: &columns,
                rowid,
                values,
            })
        })
        .collect();
    problems.extend(rows.problems().iter().map(|p| format!("{table}: {p}")));
    converted
}

/// Every row of `table`, each converted with the entry of `others` its
/// `column` names (`others` keyed by the id of `other_table`'s rows). A row
/// whose entry is missing (deleted, or never there) is kept, converted
/// with an empty one, and reported.
pub(crate) fn read_joined<P: Default, T>(
    db: &Database<'_>,
    (table, column): (&str, &str),
    (other_table, others): (&str, &HashMap<i64, P>),
    problems: &mut Vec<String>,
    convert: impl Fn(&Named<'_>, &P) -> T,
) -> Vec<T> {
    let missing = P::default();
    let mut orphans = Vec::new();
    let rows = read(db, table, problems, |row| {
        let other = row.integer(column).and_then(|id| others.get(&id));
        if other.is_none() {
            orphans.push(row.rowid);
        }
        convert(row, other.unwrap_or(&missing))
    });
    problems.extend(
        orphans
            .into_iter()
            .map(|id| format!("{table}: row {id}'s {column} is not in {other_table}")),
    );
    rows
}
