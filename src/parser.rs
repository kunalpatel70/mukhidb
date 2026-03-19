use crate::types::{Column, CompOp, DataType, Expr, Value};

/// All the SQL statements your DB understands (so far).
#[derive(Debug, PartialEq)]
pub enum Statement {
    CreateTable {
        table_name: String,
        columns: Vec<Column>,
    },
    Insert {
        table_name: String,
        values: Vec<Value>,
    },
    Select {
        table_name: String,
        where_clause: Option<Expr>,
    },
    Unknown(String),
}

/// Parse a raw SQL string into a Statement.
pub fn parse(input: &str) -> Statement {
    let input = input.trim();
    let upper = input.to_uppercase();

    if upper.starts_with("CREATE TABLE") {
        parse_create_table(input)
    } else if upper.starts_with("INSERT INTO") {
        parse_insert(input)
    } else if upper.starts_with("SELECT") {
        parse_select(input)
    } else {
        Statement::Unknown(input.to_string())
    }
}

// --- Internal parsers -----------------------------------------------------------

fn parse_create_table(input: &str) -> Statement {
    // Expected: CREATE TABLE <name> (<col> <type>, ...)
    // e.g.    : CREATE TABLE users (id INTEGER, name TEXT)
    let rest = input["CREATE TABLE".len()..].trim();
    let paren_start = rest.find('(').unwrap_or(rest.len());
    let table_name = rest[..paren_start].trim().to_string();

    let cols_str = rest
        .get(paren_start + 1..rest.rfind(')').unwrap_or(rest.len()))
        .unwrap_or("")
        .trim();

    let columns = cols_str
        .split(',')
        .filter_map(|col_def| {
            let parts: Vec<&str> = col_def.trim().splitn(2, ' ').collect();
            if parts.len() == 2 {
                let data_type = match parts[1].trim().to_uppercase().as_str() {
                    "INTEGER" => DataType::Integer,
                    "TEXT"    => DataType::Text,
                    _         => return None,
                };
                Some(Column { name: parts[0].to_string(), data_type })
            } else {
                None
            }
        })
        .collect();

    Statement::CreateTable { table_name, columns }
}

fn parse_insert(input: &str) -> Statement {
    // Expected: INSERT INTO <name> VALUES (<val>, ...)
    // e.g.    : INSERT INTO users VALUES (1, 'Alice')
    let rest = input["INSERT INTO".len()..].trim();
    let values_keyword = rest.to_uppercase().find("VALUES").unwrap_or(rest.len());
    let table_name = rest[..values_keyword].trim().to_string();

    let paren_start = rest.find('(').unwrap_or(rest.len());
    let paren_end   = rest.rfind(')').unwrap_or(rest.len());
    let vals_str    = rest.get(paren_start + 1..paren_end).unwrap_or("").trim();

    let values = vals_str
        .split(',')
        .map(|v| {
            let v = v.trim();
            if let Ok(n) = v.parse::<i64>() {
                Value::Integer(n)
            } else {
                // Strip surrounding quotes if present
                let text = v.trim_matches('\'').trim_matches('"').to_string();
                Value::Text(text)
            }
        })
        .collect();

    Statement::Insert { table_name, values }
}

fn parse_select(input: &str) -> Statement {
    // Expected: SELECT * FROM <name> [WHERE <col> <op> <val>]
    let upper = input.to_uppercase();
    let from_pos = upper.find("FROM").unwrap_or(input.len());
    let after_from = input[from_pos + "FROM".len()..].trim();

    // Split on WHERE (case-insensitive)
    let after_upper = after_from.to_uppercase();
    let (table_name, where_clause) = if let Some(w) = after_upper.find("WHERE") {
        let tname = after_from[..w].trim().to_string();
        let cond = after_from[w + "WHERE".len()..].trim();
        (tname, parse_where(cond))
    } else {
        (after_from.to_string(), None)
    };

    Statement::Select { table_name, where_clause }
}

fn parse_where(cond: &str) -> Option<Expr> {
    // Try two-char operators first, then single-char
    let ops: &[(&str, CompOp)] = &[("=", CompOp::Eq), (">", CompOp::Gt), ("<", CompOp::Lt)];
    for &(sym, ref op) in ops {
        if let Some(pos) = cond.find(sym) {
            let col = cond[..pos].trim().to_string();
            let raw = cond[pos + sym.len()..].trim();
            let value = if let Ok(n) = raw.parse::<i64>() {
                Value::Integer(n)
            } else {
                Value::Text(raw.trim_matches('\'').trim_matches('"').to_string())
            };
            return Some(Expr { column: col, op: op.clone(), value });
        }
    }
    None
}
