use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct CandidateRequest {
    pub id: u64,
    pub input: String,
    pub limit: usize,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CandidateResponse {
    pub id: u64,
    pub candidates: Vec<String>,
    pub elapsed_us: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
