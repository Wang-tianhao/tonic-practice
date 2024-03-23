use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Location {
    pub id: i32,
    pub name: String,
    pub longitude: i32,
    pub latitude: i32,
}
