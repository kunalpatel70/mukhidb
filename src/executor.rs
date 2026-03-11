use crate::parser::Statement;
use crate::storage::Storage;

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

        Statement::Select { table_name } => {
            match storage.select_all(&table_name) {
                Ok((headers, rows)) => {
                    let string_rows = rows
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
