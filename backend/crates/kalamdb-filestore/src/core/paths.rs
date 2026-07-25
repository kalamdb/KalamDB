use crate::error::{FilestoreError, Result};

/// Parse a remote URL like `s3://bucket/prefix` into (bucket, prefix).
#[cfg_attr(
    not(any(feature = "cloud-aws", feature = "cloud-gcp", feature = "cloud-azure")),
    allow(dead_code)
)]
pub(crate) fn parse_remote_url(url: &str, schemes: &[&str]) -> Result<(String, String)> {
    let trimmed = url.trim();

    for scheme in schemes {
        if let Some(rest) = trimmed.strip_prefix(scheme) {
            let (bucket, prefix) = match rest.split_once('/') {
                Some((b, p)) => (b.to_string(), p.to_string()),
                None => (rest.to_string(), String::new()),
            };
            return Ok((bucket, prefix));
        }
    }

    Err(FilestoreError::Config(format!(
        "Expected URL with schemes {:?}, got: {}",
        schemes, url
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_remote_url_s3() {
        let (bucket, prefix) = parse_remote_url("s3://my-bucket/some/prefix", &["s3://"]).unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(prefix, "some/prefix");
    }

    #[test]
    fn test_parse_remote_url_no_prefix() {
        let (bucket, prefix) = parse_remote_url("s3://my-bucket", &["s3://"]).unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(prefix, "");
    }

    #[test]
    fn test_parse_remote_url_with_trailing_slash() {
        let (bucket, prefix) = parse_remote_url("s3://my-bucket/prefix/", &["s3://"]).unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(prefix, "prefix/");
    }

    #[test]
    fn test_parse_remote_url_deep_prefix() {
        let (bucket, prefix) = parse_remote_url("s3://my-bucket/a/b/c/d/e", &["s3://"]).unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(prefix, "a/b/c/d/e");
    }

    #[test]
    fn test_parse_remote_url_invalid_scheme() {
        let result = parse_remote_url("http://bucket/key", &["s3://", "gs://"]);
        assert!(result.is_err(), "Should error on invalid scheme");
    }
}
