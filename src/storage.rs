use std::collections::HashMap;
use crate::types::{Column, DataType, Row, Table, Value};

/// The database engine — holds all tables in memory.
/// (Milestone 4 will replace this with on-disk storage.)
pub struct Storage {
    tables: HashMap<String, Table>,
}

impl Storage {
    pub fn new() -> Self {
        Storage { tables: HashMap::new() }
    }

    /// Create a new empty table with the given schema.
    pub fn create_table(&mut self, name: String, columns: Vec<Column>) -> Result<(), String> {
        if self.tables.contains_key(&name) {
            return Err(format!("Table '{}' already exists.", name));
        }
        self.tables.insert(name.clone(), Table::new(name, columns));
        Ok(())
    }

    /// Insert a row of values into a table.
    pub fn insert(&mut self, table_name: &str, values: Vec<Value>) -> Result<(), String> {
        let table = self.tables
            .get_mut(table_name)
            .ok_or_else(|| format!("Table '{}' not found.", table_name))?;

        if values.len() != table.columns.len() {
            return Err(format!(
                "Expected {} values, got {}.",
                table.columns.len(), values.len()
            ));
        }

        for (i, (col, val)) in table.columns.iter().zip(values.iter()).enumerate() {
            match (&col.data_type, val) {
                (DataType::Integer, Value::Integer(_)) => {}
                (DataType::Text, Value::Text(_)) => {}
                _ => return Err(format!(
                    "Column '{}' (position {}) expects {:?}, got {:?}.",
                    col.name, i, col.data_type, val
                )),
            }
        }

        table.rows.push(Row { values });
        Ok(())
    }

    /// Return all rows from a table (SELECT *).
    pub fn select_all(&self, table_name: &str) -> Result<(Vec<String>, Vec<&Row>), String> {
        let table = self.tables
            .get(table_name)
            .ok_or_else(|| format!("Table '{}' not found.", table_name))?;

        let headers: Vec<String> = table.columns.iter().map(|c| c.name.clone()).collect();
        let rows: Vec<&Row>      = table.rows.iter().collect();
        Ok((headers, rows))
    }
}
