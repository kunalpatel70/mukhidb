use std::fs;

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
