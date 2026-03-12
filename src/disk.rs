// Known limitation: TEXT values containing '|' are disallowed because '|' is
// used as the column delimiter in the on-disk format. An INSERT with such a
// value will return an error.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use crate::types::{Column, DataType, Row, Table, Value};

/// Encode a single Value as `<type_tag>:<data>`.
fn encode_value(v: &Value) -> Result<String, String> {
    match v {
        Value::Integer(n) => Ok(format!("int:{}", n)),
        Value::Text(s) => {
            if s.contains('|') {
                return Err("TEXT values cannot contain '|' (delimiter conflict).".into());
            }
            Ok(format!("text:{}", s))
        }
        Value::Null => Ok("null:".into()),
    }
}

/// Decode a `<type_tag>:<data>` token back into a Value.
fn decode_value(token: &str) -> Result<Value, String> {
    let (tag, data) = token.split_once(':').ok_or("Invalid value encoding")?;
    match tag {
        "int" => data.parse::<i64>().map(Value::Integer).map_err(|e| e.to_string()),
        "text" => Ok(Value::Text(data.to_string())),
        "null" => Ok(Value::Null),
        _ => Err(format!("Unknown type tag: {}", tag)),
    }
}

/// Encode the schema as a header line, e.g. `col:id:integer|col:name:text`.
fn encode_header(columns: &[Column]) -> String {
    columns
        .iter()
        .map(|c| {
            let dt = match c.data_type {
                DataType::Integer => "integer",
                DataType::Text => "text",
            };
            format!("col:{}:{}", c.name, dt)
        })
        .collect::<Vec<_>>()
        .join("|")
}

/// Decode a header line back into a Vec<Column>.
fn decode_header(line: &str) -> Result<Vec<Column>, String> {
    line.split('|')
        .map(|token| {
            let parts: Vec<&str> = token.splitn(3, ':').collect();
            if parts.len() != 3 || parts[0] != "col" {
                return Err(format!("Invalid header token: {}", token));
            }
            let data_type = match parts[2] {
                "integer" => DataType::Integer,
                "text" => DataType::Text,
                _ => return Err(format!("Unknown data type: {}", parts[2])),
            };
            Ok(Column { name: parts[1].to_string(), data_type })
        })
        .collect()
}

/// Create an empty `.db` file with a schema header line.
pub fn create_file(table_name: &str, columns: &[Column]) -> Result<(), String> {
    let path = format!("{}.db", table_name);
    let mut f = File::create(&path).map_err(|e| e.to_string())?;
    writeln!(f, "{}", encode_header(columns)).map_err(|e| e.to_string())
}

/// Append one encoded row to the table's `.db` file.
pub fn save_row(table_name: &str, row: &Row) -> Result<(), String> {
    let encoded: Result<Vec<_>, _> = row.values.iter().map(encode_value).collect();
    let line = encoded?.join("|");
    let mut f = OpenOptions::new()
        .append(true)
        .open(format!("{}.db", table_name))
        .map_err(|e| e.to_string())?;
    writeln!(f, "{}", line).map_err(|e| e.to_string())
}

/// Load a table (schema + rows) from a `.db` file. Returns None if the file
/// has no header or is empty.
pub fn load_table(path: &str) -> Result<Table, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut lines = BufReader::new(file).lines();

    let header = lines.next().ok_or("Empty file")?.map_err(|e| e.to_string())?;
    let columns = decode_header(&header)?;

    let table_name = path.trim_end_matches(".db").to_string();
    let mut table = Table::new(table_name, columns);

    for line in lines {
        let line = line.map_err(|e| e.to_string())?;
        if line.is_empty() { continue; }
        let values: Result<Vec<Value>, _> = line.split('|').map(decode_value).collect();
        table.rows.push(Row { values: values? });
    }

    Ok(table)
}

/// Scan the current directory for `.db` files and return their paths.
pub fn find_db_files() -> Vec<String> {
    fs::read_dir(".")
        .unwrap_or_else(|_| fs::read_dir(".").unwrap())
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let name = path.file_name()?.to_str()?.to_string();
            if name.ends_with(".db") { Some(name) } else { None }
        })
        .collect()
}
