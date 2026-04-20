use crate::types::{Column, CompOp, DataType, Expr, JoinClause, Value};

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
        join: Option<JoinClause>,
        where_clause: Option<Expr>,
    },
    Begin,
    Commit,
    Rollback,
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
    } else if upper == "BEGIN" {
        Statement::Begin
    } else if upper == "COMMIT" {
        Statement::Commit
    } else if upper == "ROLLBACK" {
        Statement::Rollback
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
    // Expected: SELECT * FROM <name> [JOIN <name> ON <col> = <col>] [WHERE <col> <op> <val>]
    let upper = input.to_uppercase();
    let from_pos = upper.find("FROM").unwrap_or(input.len());
    let after_from = input[from_pos + "FROM".len()..].trim();
    let after_upper = after_from.to_uppercase();

    // Check for JOIN
    let (table_name, join, remainder) = if let Some(j) = after_upper.find("JOIN") {
        let tname = after_from[..j].trim().to_string();
        let after_join = after_from[j + "JOIN".len()..].trim();
        let aj_upper = after_join.to_uppercase();

        // Find ON keyword
        let on_pos = aj_upper.find(" ON ").unwrap_or(after_join.len());
        let right_table = after_join[..on_pos].trim().to_string();
        let after_on = after_join.get(on_pos + 4..).unwrap_or("").trim();

        // Parse ON <left_col> = <right_col>, stop at WHERE if present
        let ao_upper = after_on.to_uppercase();
        let (on_cond, rest) = if let Some(w) = ao_upper.find("WHERE") {
            (after_on[..w].trim(), after_on[w..].trim())
        } else {
            (after_on, "")
        };

        // Split on '='
        let join = if let Some(eq) = on_cond.find('=') {
            let left_col = on_cond[..eq].trim().to_string();
            let right_col = on_cond[eq + 1..].trim().to_string();
            Some(JoinClause { right_table, left_col, right_col })
        } else {
            None
        };
        (tname, join, rest.to_string())
    } else {
        (String::new(), None, after_from.to_string())
    };

    // If no JOIN, table_name is everything before WHERE
    let (table_name, where_clause) = if join.is_none() {
        let rem_upper = remainder.to_uppercase();
        if let Some(w) = rem_upper.find("WHERE") {
            let tname = remainder[..w].trim().to_string();
            let cond = remainder[w + "WHERE".len()..].trim();
            (tname, parse_where(cond))
        } else {
            (remainder.trim().to_string(), None)
        }
    } else {
        // JOIN present — parse WHERE from remainder
        let rem_upper = remainder.to_uppercase();
        let where_clause = if let Some(w) = rem_upper.find("WHERE") {
            parse_where(remainder[w + "WHERE".len()..].trim())
        } else {
            None
        };
        (table_name, where_clause)
    };

    Statement::Select { table_name, join, where_clause }
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
