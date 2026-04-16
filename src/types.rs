/// The data types a column can hold.
#[derive(Debug, Clone, PartialEq)]
pub enum DataType {
    Integer,
    Text,
}

/// A single value stored in a cell.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Integer(i64),
    Text(String),
    #[allow(dead_code)] // reserved for future NULL support
    Null,
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Integer(n) => write!(f, "{}", n),
            Value::Text(s)    => write!(f, "{}", s),
            Value::Null       => write!(f, "NULL"),
        }
    }
}

/// A column definition (name + type).
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub name: String,
    pub data_type: DataType,
}

/// A single row of values.
#[derive(Debug, Clone)]
pub struct Row {
    pub values: Vec<Value>,
}

/// Comparison operators for WHERE clauses.
#[derive(Debug, Clone, PartialEq)]
pub enum CompOp {
    Eq,
    Lt,
    Gt,
}

/// A single WHERE condition: column <op> value.
#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub column: String,
    pub op: CompOp,
    pub value: Value,
}

/// JOIN clause: FROM <left> JOIN <right> ON <left_col> = <right_col>
#[derive(Debug, Clone, PartialEq)]
pub struct JoinClause {
    pub right_table: String,
    pub left_col: String,
    pub right_col: String,
}

