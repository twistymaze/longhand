//! Migrations, on Handy's `rusqlite_migration` harness
//! (`reference-source/Handy/src-tauri/src/managers/history.rs`), which tracks
//! the applied version in SQLite's `user_version` pragma.

use rusqlite_migration::{M, Migrations};

pub fn migrations() -> Migrations<'static> {
    Migrations::new(vec![M::up(include_str!("../migrations/0001_init.sql"))])
}

#[cfg(test)]
mod tests {
    #[test]
    fn migrations_are_valid() {
        super::migrations().validate().unwrap();
    }
}
