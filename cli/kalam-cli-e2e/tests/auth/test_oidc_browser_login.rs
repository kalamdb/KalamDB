use std::collections::HashMap;

use kalam_cli::session::oidc_browser::build_authorization_request;
use openidconnect::{
    core::{CoreClient, CoreJsonWebKeySet},
    AuthUrl, AuthorizationCode, ClientId, IssuerUrl, RedirectUrl, TokenUrl,
};
use url::Url;

#[test]
fn oidc_browser_authorization_url_uses_code_flow_with_pkce() {
    let client = CoreClient::new(
        ClientId::new("kalam-cli".to_string()),
        IssuerUrl::new("https://issuer.example".to_string()).unwrap(),
        CoreJsonWebKeySet::new(Vec::new()),
    )
    .set_auth_uri(AuthUrl::new("https://issuer.example/authorize".to_string()).unwrap())
    .set_token_uri(TokenUrl::new("https://issuer.example/token".to_string()).unwrap())
    .set_redirect_uri(RedirectUrl::new("http://localhost:8787/callback".to_string()).unwrap());
    let scopes = vec![
        "openid".to_string(),
        "email".to_string(),
        "profile".to_string(),
    ];

    let request = build_authorization_request(&client, &scopes);
    let query: HashMap<String, String> = Url::parse(&request.authorization_url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();

    assert_eq!(query.get("response_type"), Some(&"code".to_string()));
    assert_eq!(query.get("client_id"), Some(&"kalam-cli".to_string()));
    assert_eq!(query.get("redirect_uri"), Some(&"http://localhost:8787/callback".to_string()));
    assert_eq!(query.get("code_challenge_method"), Some(&"S256".to_string()));
    assert!(query.get("code_challenge").is_some_and(|value| !value.is_empty()));
    assert!(query.get("state").is_some_and(|value| !value.is_empty()));
    assert!(query.get("nonce").is_some_and(|value| !value.is_empty()));

    let scope = query.get("scope").expect("scope should be present");
    assert!(scope.contains("openid"));
    assert!(scope.contains("email"));
    assert!(scope.contains("profile"));
    assert_ne!(request.csrf_state.secret(), "");
    assert_ne!(request.nonce.secret(), "");
    assert!(request.pkce_verifier.secret().len() >= 43);

    let _code_exchange = client
        .exchange_code(AuthorizationCode::new("authorization-code".to_string()))
        .set_pkce_verifier(request.pkce_verifier);
}
