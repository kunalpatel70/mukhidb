use crate::parser::Statement;
use crate::storage::Storage;
use crate::types::{CompOp, Expr, Row, Value};

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

        Statement::Select { table_name, where_clause } => {
            match storage.select_all(&table_name) {
                Ok((headers, rows)) => {
                    let filtered: Vec<&Row> = rows
                        .iter()
                        .filter(|row| match &where_clause {
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

        Statement::Unknown(cmd) => {
            ExecuteResult::Message(format!("Unrecognised command: '{}'", cmd))
        }
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
