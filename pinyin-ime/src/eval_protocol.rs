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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_roundtrip_preserves_unicode_candidates() {
        let response = CandidateResponse {
            id: 7,
            candidates: vec!["你好".to_string(), "👋".to_string()],
            elapsed_us: 42,
            error: None,
        };
        let encoded = serde_json::to_string(&response).unwrap();
        let decoded: CandidateResponse = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.id, 7);
        assert_eq!(decoded.candidates, ["你好", "👋"]);
    }
}
