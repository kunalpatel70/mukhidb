use crate::parser::Statement;
use crate::storage::Storage;
use crate::types::{CompOp, Expr, JoinClause, Row, Value};

/// The result of executing a statement — either a message or a table of rows.
pub enum ExecuteResult {
    Message(String),
    Rows { headers: Vec<String>, rows: Vec<Vec<String>> },
}

/// Execute a parsed Statement against the Storage layer.
pub fn execute(statement: Statement, storage: &mut Storage) -> ExecuteResult {
    match statement {
        Statement::CreateTable { table_name, columns } => {
            match storage.create_table(table_name.clone(), columns) {
                Ok(_)    => ExecuteResult::Message(format!("Table '{}' created.", table_name)),
                Err(msg) => ExecuteResult::Message(format!("Error: {}", msg)),
            }
        }

        Statement::Insert { table_name, values } => {
            match storage.insert(&table_name, values) {
                Ok(_)    => ExecuteResult::Message("1 row inserted.".to_string()),
                Err(msg) => ExecuteResult::Message(format!("Error: {}", msg)),
            }
        }

        Statement::Select { table_name, join, where_clause } => {
            if let Some(ref jc) = join {
                execute_join(&table_name, jc, &where_clause, storage)
            } else {
                execute_select(&table_name, &where_clause, storage)
            }
        }

        Statement::Unknown(cmd) => {
            ExecuteResult::Message(format!("Unrecognised command: '{}'", cmd))
        }

        Statement::Begin => match storage.begin() {
            Ok(_) => ExecuteResult::Message("Transaction started.".to_string()),
            Err(e) => ExecuteResult::Message(format!("Error: {}", e)),
        },

        Statement::Commit => match storage.commit() {
            Ok(_) => ExecuteResult::Message("Transaction committed.".to_string()),
            Err(e) => ExecuteResult::Message(format!("Error: {}", e)),
        },

        Statement::Rollback => match storage.rollback() {
            Ok(_) => ExecuteResult::Message("Transaction rolled back.".to_string()),
            Err(e) => ExecuteResult::Message(format!("Error: {}", e)),
        },
    }
}

/// Evaluate a WHERE expression against a single row.
fn evaluate(expr: &Expr, row: &Row, headers: &[String]) -> bool {
    let idx = match headers.iter().position(|h| h.eq_ignore_ascii_case(&expr.column)) {
        Some(i) => i,
        None => return false,
    };
    let cell = &row.values[idx];
    match (&expr.op, cell, &expr.value) {
        (CompOp::Eq, Value::Integer(a), Value::Integer(b)) => a == b,
        (CompOp::Lt, Value::Integer(a), Value::Integer(b)) => a < b,
        (CompOp::Gt, Value::Integer(a), Value::Integer(b)) => a > b,
        (CompOp::Eq, Value::Text(a), Value::Text(b)) => a == b,
        (CompOp::Lt, Value::Text(a), Value::Text(b)) => a < b,
        (CompOp::Gt, Value::Text(a), Value::Text(b)) => a > b,
        _ => false,
    }
}

fn execute_select(
    table_name: &str,
    where_clause: &Option<Expr>,
    storage: &mut Storage,
) -> ExecuteResult {
    match storage.select_all(table_name) {
        Ok((headers, rows)) => {
            let filtered: Vec<&Row> = rows
                .iter()
                .filter(|row| match where_clause {
                    Some(expr) => evaluate(expr, row, &headers),
                    None => true,
                })
                .collect();
            let string_rows = filtered
                .iter()
                .map(|row| row.values.iter().map(|v| v.to_string()).collect())
                .collect();
            ExecuteResult::Rows { headers, rows: string_rows }
        }
        Err(msg) => ExecuteResult::Message(format!("Error: {}", msg)),
    }
}

fn execute_join(
    left_table: &str,
    jc: &JoinClause,
    where_clause: &Option<Expr>,
    storage: &mut Storage,
) -> ExecuteResult {
    // Fetch both tables.
    let (left_hdrs, left_rows) = match storage.select_all(left_table) {
        Ok(r) => r,
        Err(e) => return ExecuteResult::Message(format!("Error: {}", e)),
    };
    let (right_hdrs, right_rows) = match storage.select_all(&jc.right_table) {
        Ok(r) => r,
        Err(e) => return ExecuteResult::Message(format!("Error: {}", e)),
    };

    // Resolve join column indices. Support both "col" and "table.col" syntax.
    let left_idx = find_col_index(&left_hdrs, left_table, &jc.left_col);
    let right_idx = find_col_index(&right_hdrs, &jc.right_table, &jc.right_col);

    let (left_idx, right_idx) = match (left_idx, right_idx) {
        (Some(l), Some(r)) => (l, r),
        _ => return ExecuteResult::Message("Error: JOIN column not found.".to_string()),
    };

    // Build prefixed headers: left_table.col, right_table.col
    let mut headers: Vec<String> = left_hdrs.iter().map(|h| format!("{}.{}", left_table, h)).collect();
    headers.extend(right_hdrs.iter().map(|h| format!("{}.{}", jc.right_table, h)));

    // Nested loop join.
    let mut joined_rows: Vec<Row> = Vec::new();
    for lr in &left_rows {
        for rr in &right_rows {
            if lr.values[left_idx] == rr.values[right_idx] {
                let mut vals = lr.values.clone();
                vals.extend(rr.values.clone());
                joined_rows.push(Row { values: vals });
            }
        }
    }

    // Apply WHERE filter on joined result.
    let filtered: Vec<&Row> = joined_rows
        .iter()
        .filter(|row| match where_clause {
            Some(expr) => evaluate(expr, row, &headers),
            None => true,
        })
        .collect();

    let string_rows = filtered
        .iter()
        .map(|row| row.values.iter().map(|v| v.to_string()).collect())
        .collect();
    ExecuteResult::Rows { headers, rows: string_rows }
}

/// Find a column index, supporting both bare "col" and "table.col" syntax.
fn find_col_index(headers: &[String], _table: &str, col_ref: &str) -> Option<usize> {
    // If col_ref contains '.', use the part after the dot.
    let col_name = if let Some(dot) = col_ref.find('.') {
        &col_ref[dot + 1..]
    } else {
        col_ref
    };
    headers.iter().position(|h| h.eq_ignore_ascii_case(col_name))
}
