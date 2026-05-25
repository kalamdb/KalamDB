//! Well-known OAuth / OIDC identity providers.
//!
//! Each provider has a deterministic 3-character prefix used in
//! `oidc:{prefix}:{subject}` usernames. This lives in `kalamdb-commons`
//! so that shared auth models and system auth data can
//! reference it without circular dependencies.

use std::fmt;

use sha2::{Digest, Sha256};

struct OAuthProviderMetadata {
    provider: OAuthProvider,
    canonical: &'static str,
    prefix: &'static str,
    aliases: &'static [&'static str],
    issuer_patterns: &'static [&'static str],
    issuer_requires_all: bool,
}

const OAUTH_PROVIDER_METADATA: &[OAuthProviderMetadata] = &[
    OAuthProviderMetadata {
        provider: OAuthProvider::Keycloak,
        canonical: "keycloak",
        prefix: "kcl",
        aliases: &["keycloak"],
        issuer_patterns: &["keycloak", "/realms/"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::Google,
        canonical: "google",
        prefix: "ggl",
        aliases: &["google"],
        issuer_patterns: &["accounts.google.com"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::AzureAd,
        canonical: "azure_ad",
        prefix: "msf",
        aliases: &["azure_ad", "azure", "microsoft"],
        issuer_patterns: &["login.microsoftonline.com", "sts.windows.net"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::Cognito,
        canonical: "cognito",
        prefix: "cgn",
        aliases: &["cognito", "aws_cognito"],
        issuer_patterns: &["cognito-idp", "amazonaws.com"],
        issuer_requires_all: true,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::AwsIam,
        canonical: "aws_iam",
        prefix: "aws",
        aliases: &["aws_iam"],
        issuer_patterns: &[],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::GitHub,
        canonical: "github",
        prefix: "ghb",
        aliases: &["github"],
        issuer_patterns: &["github.com"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::GitLab,
        canonical: "gitlab",
        prefix: "glb",
        aliases: &["gitlab"],
        issuer_patterns: &["gitlab.com", "gitlab"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::Facebook,
        canonical: "facebook",
        prefix: "fbk",
        aliases: &["facebook", "meta"],
        issuer_patterns: &["facebook.com"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::X,
        canonical: "x",
        prefix: "xco",
        aliases: &["x", "twitter"],
        issuer_patterns: &["twitter.com", "x.com"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::Apple,
        canonical: "apple",
        prefix: "apl",
        aliases: &["apple"],
        issuer_patterns: &["appleid.apple.com"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::Firebase,
        canonical: "firebase",
        prefix: "fbs",
        aliases: &["firebase", "google_identity_platform"],
        issuer_patterns: &["securetoken.google.com"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::Okta,
        canonical: "okta",
        prefix: "okt",
        aliases: &["okta"],
        issuer_patterns: &["okta.com", "oktapreview.com"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::Auth0,
        canonical: "auth0",
        prefix: "a0x",
        aliases: &["auth0"],
        issuer_patterns: &["auth0.com"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::Supabase,
        canonical: "supabase",
        prefix: "sbs",
        aliases: &["supabase"],
        issuer_patterns: &["supabase"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::OneLogin,
        canonical: "onelogin",
        prefix: "olg",
        aliases: &["onelogin"],
        issuer_patterns: &["onelogin.com"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::PingIdentity,
        canonical: "ping_identity",
        prefix: "png",
        aliases: &["ping_identity", "ping", "pingfederate"],
        issuer_patterns: &["pingidentity.com", "pingone.com"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::Salesforce,
        canonical: "salesforce",
        prefix: "sfc",
        aliases: &["salesforce"],
        issuer_patterns: &["salesforce.com", "force.com"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::Oracle,
        canonical: "oracle",
        prefix: "orc",
        aliases: &["oracle"],
        issuer_patterns: &["identity.oraclecloud.com"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::Ibm,
        canonical: "ibm",
        prefix: "ibm",
        aliases: &["ibm"],
        issuer_patterns: &["verify.ibm.com"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::JumpCloud,
        canonical: "jumpcloud",
        prefix: "jcl",
        aliases: &["jumpcloud"],
        issuer_patterns: &["jumpcloud.com"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::Duo,
        canonical: "duo",
        prefix: "duo",
        aliases: &["duo"],
        issuer_patterns: &["duosecurity.com"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::FusionAuth,
        canonical: "fusionauth",
        prefix: "fsa",
        aliases: &["fusionauth"],
        issuer_patterns: &["fusionauth"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::Authentik,
        canonical: "authentik",
        prefix: "atk",
        aliases: &["authentik"],
        issuer_patterns: &["authentik"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::Zitadel,
        canonical: "zitadel",
        prefix: "zit",
        aliases: &["zitadel"],
        issuer_patterns: &["zitadel"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::Casdoor,
        canonical: "casdoor",
        prefix: "csd",
        aliases: &["casdoor"],
        issuer_patterns: &["casdoor"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::Logto,
        canonical: "logto",
        prefix: "lgt",
        aliases: &["logto"],
        issuer_patterns: &["logto"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::Clerk,
        canonical: "clerk",
        prefix: "clk",
        aliases: &["clerk"],
        issuer_patterns: &["clerk"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::Stytch,
        canonical: "stytch",
        prefix: "sty",
        aliases: &["stytch"],
        issuer_patterns: &["stytch.com"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::WorkOS,
        canonical: "workos",
        prefix: "wos",
        aliases: &["workos"],
        issuer_patterns: &["workos.com"],
        issuer_requires_all: false,
    },
    OAuthProviderMetadata {
        provider: OAuthProvider::Descope,
        canonical: "descope",
        prefix: "dsc",
        aliases: &["descope"],
        issuer_patterns: &["descope.com"],
        issuer_requires_all: false,
    },
];

fn provider_metadata(provider: &OAuthProvider) -> Option<&'static OAuthProviderMetadata> {
    OAUTH_PROVIDER_METADATA.iter().find(|metadata| metadata.provider == *provider)
}

fn provider_from_prefix(prefix: &str) -> Option<OAuthProvider> {
    OAUTH_PROVIDER_METADATA
        .iter()
        .find(|metadata| metadata.prefix == prefix)
        .map(|metadata| metadata.provider.clone())
}

fn provider_from_alias(alias: &str) -> Option<OAuthProvider> {
    OAUTH_PROVIDER_METADATA
        .iter()
        .find(|metadata| metadata.aliases.contains(&alias))
        .map(|metadata| metadata.provider.clone())
}

fn provider_from_issuer(lower_issuer: &str) -> Option<OAuthProvider> {
    OAUTH_PROVIDER_METADATA
        .iter()
        .find(|metadata| {
            !metadata.issuer_patterns.is_empty()
                && if metadata.issuer_requires_all {
                    metadata.issuer_patterns.iter().all(|pattern| lower_issuer.contains(pattern))
                } else {
                    metadata.issuer_patterns.iter().any(|pattern| lower_issuer.contains(pattern))
                }
        })
        .map(|metadata| metadata.provider.clone())
}

fn custom_provider_prefix(identifier: &str) -> String {
    let hash = hex::encode(Sha256::digest(identifier.as_bytes()));
    hash[..3].to_string()
}

/// Well-known OAuth / OIDC identity providers.
///
/// Serialises as a lowercase snake_case string.  Unknown provider strings
/// deserialize into [`Custom`](OAuthProvider::Custom) so that new providers
/// can be added at the identity-provider side without requiring a KalamDB
/// code change.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum OAuthProvider {
    /// Keycloak (any realm)
    Keycloak,
    /// Google / Google Workspace
    Google,
    /// Microsoft Azure Active Directory / Entra ID
    AzureAd,
    /// Amazon Cognito
    Cognito,
    /// AWS IAM Identity Center (successor to AWS SSO)
    AwsIam,
    /// GitHub OAuth / GitHub Apps
    GitHub,
    /// GitLab (self-hosted or gitlab.com)
    GitLab,
    /// Meta / Facebook Login
    Facebook,
    /// X (formerly Twitter)
    X,
    /// Sign in with Apple
    Apple,
    /// Firebase Authentication (backed by Google Identity Platform)
    Firebase,
    /// Okta / Okta Workforce Identity Cloud
    Okta,
    /// Auth0 (by Okta)
    Auth0,
    /// Supabase Auth (GoTrue-based)
    Supabase,
    /// OneLogin
    OneLogin,
    /// Ping Identity / PingFederate
    PingIdentity,
    /// Salesforce Identity
    Salesforce,
    /// Oracle Identity Cloud Service
    Oracle,
    /// IBM Security Verify
    Ibm,
    /// JumpCloud
    JumpCloud,
    /// Duo Security
    Duo,
    /// FusionAuth
    FusionAuth,
    /// Authentik (open-source)
    Authentik,
    /// Zitadel (open-source)
    Zitadel,
    /// Casdoor (open-source)
    Casdoor,
    /// Logto (open-source)
    Logto,
    /// Clerk
    Clerk,
    /// Stytch
    Stytch,
    /// WorkOS
    WorkOS,
    /// Descope
    Descope,
    /// Any provider not in the well-known list.
    /// The contained string is the raw provider identifier.
    Custom(String),
}

impl OAuthProvider {
    /// Canonical string representation (matches the serde serialisation).
    pub fn as_str(&self) -> &str {
        provider_metadata(self)
            .map(|metadata| metadata.canonical)
            .unwrap_or_else(|| match self {
                Self::Custom(value) => value.as_str(),
                _ => unreachable!("known OAuth providers must have metadata"),
            })
    }

    /// 3-character prefix used in `oidc:{prefix}:{subject}` usernames.
    ///
    /// Well-known providers get a deterministic static prefix.
    /// Custom providers get the first 3 hex characters of the SHA-256 hash
    /// of their identifier string.
    pub fn prefix(&self) -> String {
        provider_metadata(self)
            .map(|metadata| metadata.prefix.to_string())
            .unwrap_or_else(|| match self {
                Self::Custom(value) => custom_provider_prefix(value),
                _ => unreachable!("known OAuth providers must have metadata"),
            })
    }

    /// Reverse-lookup a provider from its 3-character username prefix.
    ///
    /// Unknown prefixes return [`Custom`](Self::Custom) with the prefix
    /// as the identifier (lossy — the original issuer URL is not recoverable
    /// from a hash prefix).
    pub fn from_prefix(prefix: &str) -> Self {
        provider_from_prefix(prefix).unwrap_or_else(|| Self::Custom(prefix.to_string()))
    }

    /// Parse from a string, falling back to [`Custom`](Self::Custom) for
    /// unknown values.
    pub fn from_str_lossy(s: &str) -> Self {
        provider_from_alias(s).unwrap_or_else(|| Self::Custom(s.to_string()))
    }

    /// Detect the provider type from an OIDC issuer URL.
    ///
    /// Uses substring matching on well-known issuer URL patterns.
    /// Falls back to [`Custom`](Self::Custom) with the raw URL.
    pub fn detect_from_issuer(issuer: &str) -> Self {
        let lower = issuer.to_lowercase();
        provider_from_issuer(&lower).unwrap_or_else(|| Self::Custom(issuer.to_string()))
    }
}

impl fmt::Display for OAuthProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Conditional serde impls (behind "serde" feature)
// ---------------------------------------------------------------------------

#[cfg(feature = "serde")]
mod serde_impl {
    use super::OAuthProvider;

    impl serde::Serialize for OAuthProvider {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            serializer.serialize_str(self.as_str())
        }
    }

    impl<'de> serde::Deserialize<'de> for OAuthProvider {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let s = String::deserialize(deserializer)?;
            Ok(Self::from_str_lossy(&s))
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_oauth_provider_serde_roundtrip() {
        for provider in &[
            OAuthProvider::Keycloak,
            OAuthProvider::Google,
            OAuthProvider::AzureAd,
            OAuthProvider::GitHub,
            OAuthProvider::Auth0,
            OAuthProvider::Custom("my_idp".to_string()),
        ] {
            let json = serde_json::to_string(provider).unwrap();
            let back: OAuthProvider = serde_json::from_str(&json).unwrap();
            assert_eq!(*provider, back, "round-trip failed for {json}");
        }
    }

    #[test]
    fn test_oauth_provider_detect_keycloak() {
        let p = OAuthProvider::detect_from_issuer("https://keycloak.example.com/realms/myrealm");
        assert_eq!(p, OAuthProvider::Keycloak);
    }

    #[test]
    fn test_oauth_provider_detect_google() {
        let p = OAuthProvider::detect_from_issuer("https://accounts.google.com");
        assert_eq!(p, OAuthProvider::Google);
    }

    #[test]
    fn test_oauth_provider_detect_azure() {
        let p =
            OAuthProvider::detect_from_issuer("https://login.microsoftonline.com/tenant-id/v2.0");
        assert_eq!(p, OAuthProvider::AzureAd);
    }

    #[test]
    fn test_oauth_provider_detect_custom() {
        let p = OAuthProvider::detect_from_issuer("https://my-idp.internal.corp");
        assert_eq!(p, OAuthProvider::Custom("https://my-idp.internal.corp".to_string()));
    }

    #[test]
    fn test_oauth_provider_aliases() {
        assert_eq!(OAuthProvider::from_str_lossy("twitter"), OAuthProvider::X);
        assert_eq!(OAuthProvider::from_str_lossy("meta"), OAuthProvider::Facebook);
        assert_eq!(OAuthProvider::from_str_lossy("microsoft"), OAuthProvider::AzureAd);
        assert_eq!(OAuthProvider::from_str_lossy("aws_cognito"), OAuthProvider::Cognito);
    }

    #[test]
    fn test_prefix_roundtrip_wellknown() {
        let providers = [
            OAuthProvider::Keycloak,
            OAuthProvider::Google,
            OAuthProvider::AzureAd,
            OAuthProvider::Cognito,
            OAuthProvider::AwsIam,
            OAuthProvider::GitHub,
            OAuthProvider::GitLab,
            OAuthProvider::Facebook,
            OAuthProvider::X,
            OAuthProvider::Apple,
            OAuthProvider::Firebase,
            OAuthProvider::Okta,
            OAuthProvider::Auth0,
            OAuthProvider::Supabase,
            OAuthProvider::OneLogin,
            OAuthProvider::PingIdentity,
            OAuthProvider::Salesforce,
            OAuthProvider::Oracle,
            OAuthProvider::Ibm,
            OAuthProvider::JumpCloud,
            OAuthProvider::Duo,
            OAuthProvider::FusionAuth,
            OAuthProvider::Authentik,
            OAuthProvider::Zitadel,
            OAuthProvider::Casdoor,
            OAuthProvider::Logto,
            OAuthProvider::Clerk,
            OAuthProvider::Stytch,
            OAuthProvider::WorkOS,
            OAuthProvider::Descope,
        ];

        for provider in &providers {
            let prefix = provider.prefix();
            assert_eq!(prefix.len(), 3, "prefix for {:?} is not 3 chars", provider);
            let back = OAuthProvider::from_prefix(&prefix);
            assert_eq!(
                *provider, back,
                "prefix round-trip failed for {:?} (prefix={})",
                provider, prefix
            );
        }
    }

    #[test]
    fn test_prefix_no_duplicates() {
        let providers = [
            OAuthProvider::Keycloak,
            OAuthProvider::Google,
            OAuthProvider::AzureAd,
            OAuthProvider::Cognito,
            OAuthProvider::AwsIam,
            OAuthProvider::GitHub,
            OAuthProvider::GitLab,
            OAuthProvider::Facebook,
            OAuthProvider::X,
            OAuthProvider::Apple,
            OAuthProvider::Firebase,
            OAuthProvider::Okta,
            OAuthProvider::Auth0,
            OAuthProvider::Supabase,
            OAuthProvider::OneLogin,
            OAuthProvider::PingIdentity,
            OAuthProvider::Salesforce,
            OAuthProvider::Oracle,
            OAuthProvider::Ibm,
            OAuthProvider::JumpCloud,
            OAuthProvider::Duo,
            OAuthProvider::FusionAuth,
            OAuthProvider::Authentik,
            OAuthProvider::Zitadel,
            OAuthProvider::Casdoor,
            OAuthProvider::Logto,
            OAuthProvider::Clerk,
            OAuthProvider::Stytch,
            OAuthProvider::WorkOS,
            OAuthProvider::Descope,
        ];

        let mut prefixes: Vec<String> = providers.iter().map(|p| p.prefix()).collect();
        let original_len = prefixes.len();
        prefixes.sort();
        prefixes.dedup();
        assert_eq!(
            prefixes.len(),
            original_len,
            "duplicate prefixes found among well-known providers"
        );
    }

    #[test]
    fn test_prefix_custom_deterministic() {
        let p = OAuthProvider::Custom("https://my-idp.internal.corp".to_string());
        let prefix1 = p.prefix();
        let prefix2 = p.prefix();
        assert_eq!(prefix1, prefix2);
        assert_eq!(prefix1.len(), 3);
    }

    #[test]
    fn test_from_prefix_unknown_returns_custom() {
        let p = OAuthProvider::from_prefix("zzz");
        assert_eq!(p, OAuthProvider::Custom("zzz".to_string()));
    }
}
