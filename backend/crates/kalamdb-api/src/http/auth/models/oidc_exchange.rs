use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OidcCodeExchangeRequest {
    pub code:          String,
    pub redirect_uri:  String,
    pub code_verifier: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OidcTokenExchangeRequest {
    pub token: String,
}
