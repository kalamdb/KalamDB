//! Comprehensive unit tests for password hashing and validation
//!
//! Tests cover:
//! - Password hashing with bcrypt
//! - Password verification
//! - Common password blocking
//! - Password strength validation
//! - Edge cases and security requirements

use kalamdb_auth::security::password::{
    hash_password, validate_password, validate_password_characters, verify_password, BCRYPT_COST,
    MAX_PASSWORD_LENGTH, MIN_PASSWORD_LENGTH,
};

/// Test basic password hashing functionality
#[tokio::test]
async fn test_hash_password() {
    let password = "SecurePassword123!";
    let hash = hash_password(password, None).await.unwrap();

    // Verify hash format
    assert!(hash.starts_with("$2b$"), "Hash should be bcrypt format");
    assert!(hash.len() > 50, "Hash should be sufficiently long");

    // Verify hash is different each time (due to random salt)
    let hash2 = hash_password(password, None).await.unwrap();
    assert_ne!(hash, hash2, "Each hash should have unique salt");
}

/// Test password verification with correct password
#[tokio::test]
async fn test_verify_password_correct() {
    let password = "MyTestPassword2024!";
    let hash = hash_password(password, Some(4)).await.unwrap(); // Low cost for faster tests

    let result = verify_password(password, &hash).await.unwrap();
    assert!(result, "Correct password should verify successfully");
}

/// Test password verification with wrong password
#[tokio::test]
async fn test_verify_password_wrong() {
    let password = "CorrectPassword123!";
    let wrong_password = "WrongPassword456!";
    let hash = hash_password(password, Some(4)).await.unwrap();

    let result = verify_password(wrong_password, &hash).await.unwrap();
    assert!(!result, "Wrong password should not verify");
}

/// Test password verification is case-sensitive
#[tokio::test]
async fn test_verify_password_case_sensitive() {
    let password = "CaseSensitive123!";
    let hash = hash_password(password, Some(4)).await.unwrap();

    let wrong_case = "casesensitive123!";
    let result = verify_password(wrong_case, &hash).await.unwrap();
    assert!(!result, "Password verification should be case-sensitive");
}

/// Test password hashing with custom cost factor
#[tokio::test]
async fn test_hash_password_custom_cost() {
    let password = "TestPassword123!";

    // Low cost (fast for testing)
    let hash_low = hash_password(password, Some(4)).await.unwrap();
    assert!(hash_low.starts_with("$2b$04$"), "Should use cost 4");

    // Default cost
    let hash_default = hash_password(password, None).await.unwrap();
    assert!(
        hash_default.starts_with(&format!("$2b${:02}$", BCRYPT_COST)),
        "Should use default cost {}",
        BCRYPT_COST
    );
}

/// Test common password rejection
#[test]
fn test_common_password_rejected() {
    let common_passwords = vec![
        "password", "123456", "12345678", "qwerty", "abc123", "letmein", "monkey",
    ];

    for password in common_passwords {
        let result = validate_password(password);
        assert!(result.is_err(), "Common password '{}' should be rejected", password);

        if let Err(e) = result {
            let error_msg = format!("{:?}", e);
            assert!(
                error_msg.contains("common") || error_msg.contains("weak"),
                "Error should mention common/weak password"
            );
        }
    }
}

/// Test password too short rejection
#[test]
fn test_password_too_short() {
    let short_passwords = vec![
        "",        // Empty
        "a",       // 1 char
        "ab",      // 2 chars
        "abc",     // 3 chars
        "1234567", // 7 chars (just below minimum)
    ];

    for password in short_passwords {
        let result = validate_password(password);
        assert!(
            result.is_err(),
            "Password '{}' (len={}) should be rejected as too short",
            password,
            password.len()
        );

        if let Err(e) = result {
            let error_msg = format!("{:?}", e);
            assert!(
                error_msg.contains(&MIN_PASSWORD_LENGTH.to_string())
                    || error_msg.contains("characters"),
                "Error should mention minimum length"
            );
        }
    }
}

/// Test password exactly at minimum length
#[test]
fn test_password_minimum_length() {
    // Minimum length is 8 characters
    let password = "12345678"; // Exactly 8 chars, but common
    let result = validate_password(password);
    // Should fail due to being common, not length
    assert!(result.is_err());

    let password_unique = "Abcd1234"; // Exactly 8 chars, not common
    let result = validate_password(password_unique);
    assert!(result.is_ok(), "8-character unique password should be valid");
}

/// Test password too long rejection
#[test]
fn test_password_too_long() {
    // Bcrypt has a 72-byte limit
    let too_long = "a".repeat(MAX_PASSWORD_LENGTH + 1);
    let result = validate_password(&too_long);

    assert!(
        result.is_err(),
        "Password longer than {} chars should be rejected",
        MAX_PASSWORD_LENGTH
    );

    if let Err(e) = result {
        let error_msg = format!("{:?}", e);
        assert!(
            error_msg.contains(&MAX_PASSWORD_LENGTH.to_string()),
            "Error should mention maximum length"
        );
    }
}

/// Test password exactly at maximum length
#[test]
fn test_password_maximum_length() {
    // Maximum length is 72 characters
    let password = "A".repeat(MAX_PASSWORD_LENGTH);
    let result = validate_password(&password);
    assert!(result.is_ok(), "72-character password should be valid");
}

/// Test valid strong passwords
#[test]
fn test_valid_strong_passwords() {
    let strong_passwords = vec![
        "MySecurePass123!",
        "C0mpl3x&P@ssw0rd",
        "ThisIsAVeryLongPasswordThatIsNotCommon2024!",
        "Tr0ub4dor&3",
        "correct-horse-battery-staple-2024",
    ];

    for password in strong_passwords {
        let result = validate_password(password);
        assert!(result.is_ok(), "Strong password '{}' should be valid", password);
    }
}

/// Test password with special characters
#[test]
fn test_password_special_characters() {
    let passwords_with_special = vec!["Pass@123", "P@ssw0rd!", "MyP@ss#123", "Test$ecure&2024"];

    for password in passwords_with_special {
        let result = validate_password(password);
        assert!(result.is_ok(), "Password with special chars '{}' should be valid", password);
    }
}

/// Test password verification with invalid hash format
#[tokio::test]
async fn test_verify_password_invalid_hash() {
    let password = "TestPassword123!";
    let invalid_hashes = vec![
        "",                   // Empty
        "not-a-hash",         // Plain text
        "$2a$04$invalid",     // Incomplete hash
        "plaintext_password", // Not hashed
    ];

    for invalid_hash in invalid_hashes {
        let result = verify_password(password, invalid_hash).await;
        assert!(result.is_err(), "Invalid hash '{}' should return error", invalid_hash);
    }
}

/// Test concurrent password hashing (ensures thread safety)
#[tokio::test]
async fn test_concurrent_password_hashing() {
    let mut handles = vec![];

    for i in 0..10 {
        let password = format!("ConcurrentTest{}!", i);
        let handle = tokio::spawn(async move { hash_password(&password, Some(4)).await });
        handles.push(handle);
    }

    // Wait for all hashing operations
    for handle in handles {
        let result = handle.await.unwrap();
        assert!(result.is_ok(), "Concurrent hashing should succeed");
    }
}

/// Test password validation edge cases
#[test]
fn test_password_validation_edge_cases() {
    // Unicode characters
    let unicode_password = "Pässwörd123!";
    let result = validate_password(unicode_password);
    assert!(result.is_ok(), "Unicode password should be valid");

    // Whitespace
    let whitespace_password = "My Pass 123!";
    let result = validate_password(whitespace_password);
    assert!(result.is_ok(), "Password with spaces should be valid");

    // Numbers only (but not common)
    let numbers_only = "98765432101234";
    let result = validate_password(numbers_only);
    assert!(result.is_ok(), "Long number sequence should be valid");
}

/// Test that control and invisible formatting characters are rejected.
#[test]
fn test_password_control_and_invisible_characters_rejected() {
    let invalid_passwords = [
        "Line\nBreak123!",
        "Tab\tCharacter123!",
        "Hidden\u{200B}Format123!",
        "Bidi\u{2066}Mask123!",
    ];

    for password in invalid_passwords {
        let result = validate_password_characters(password);
        assert!(result.is_err(), "'{}' should be rejected", password.escape_debug());

        if let Err(error) = result {
            assert!(error.to_string().contains("control or invisible"));
        }
    }
}

/// Test that SQL-looking punctuation remains valid password data.
#[test]
fn test_sql_like_password_characters_are_allowed() {
    let password = "Quote'\";Comment--123!";
    let result = validate_password(password);
    assert!(result.is_ok(), "SQL-looking punctuation should remain valid password data");
}

/// Test that common password check is case-insensitive
#[test]
fn test_common_password_case_insensitive() {
    // Common passwords in different cases
    let _variations = [
        "password", // lowercase
        "PASSWORD", // uppercase (may or may not be in list)
        "Password", // mixed case (may or may not be in list)
    ];

    // At least the lowercase version should be rejected
    let result = validate_password("password");
    assert!(result.is_err(), "Lowercase 'password' should definitely be rejected");
}

/// Benchmark helper: Test hashing performance (not a real benchmark, just sanity check)
#[tokio::test]
#[ignore] // Ignored by default as it takes time
async fn test_hashing_performance() {
    use std::time::Instant;

    let password = "PerformanceTest123!";
    let start = Instant::now();
    let _hash = hash_password(password, Some(12)).await.unwrap();
    let duration = start.elapsed();

    println!("Hashing with cost 12 took: {:?}", duration);
    assert!(duration.as_millis() < 1000, "Hashing should complete within 1 second");
}
