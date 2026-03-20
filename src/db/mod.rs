pub mod loader;
pub mod query;

use duckdb::Connection;

pub fn create_connection() -> Result<Connection, duckdb::Error> {
    Connection::open_in_memory()
}
