//! Integration tests for the SSH key upload endpoint (POST /api/ssh-keys).
//!
//! Tests against the running server at BASE_URL (default: https://oxmux.app).

use anyhow::Result;

const TEST_USER: &str = "e2e_test_user";
const TEST_PASS: &str = "e2e_test_pass_1234";

/// A valid Ed25519 private key for testing (not used for any real auth).
const TEST_ED25519_KEY: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACAFUwz8BnJdEhZqSRY4uIn8u7v5gPBfRQvcHBHl6SoRPAAAAJCysYq2srGK
tgAAAAtzc2gtZWQyNTUxOQAAACAFUwz8BnJdEhZqSRY4uIn8u7v5gPBfRQvcHBHl6SoRPA
AAAEAk/b0N/ru79oshiD9K8OeT7Tz4CdXQy5gD2m4xNbcvBAVTDPwGcl0SFmpJFji4ify7
u/mA8F9FC9wcEeXpKhE8AAAACnRlc3RAb3htdXgBAgM=
-----END OPENSSH PRIVATE KEY-----";

async fn get_auth_token(client: &reqwest::Client, base_url: &str) -> Result<String> {
    let res = client
        .post(format!("{}/api/auth/login", base_url))
        .json(&serde_json::json!({
            "username": TEST_USER,
            "password": TEST_PASS,
        }))
        .send()
        .await?;

    if res.status() != 200 {
        // User may not exist yet — try register
        let res = client
            .post(format!("{}/api/auth/register", base_url))
            .json(&serde_json::json!({
                "username": TEST_USER,
                "password": TEST_PASS,
            }))
            .send()
            .await?;
        let body: serde_json::Value = res.json().await?;
        return Ok(body["token"].as_str().unwrap().to_string());
    }

    let body: serde_json::Value = res.json().await?;
    Ok(body["token"].as_str().unwrap().to_string())
}

#[tokio::test]
async fn test_upload_valid_key() -> Result<()> {
    let base_url = std::env::var("BASE_URL").unwrap_or_else(|_| "https://oxmux.app".to_string());
    let client = reqwest::Client::new();
    let token = get_auth_token(&client, &base_url).await?;

    let res = client
        .post(format!("{}/api/ssh-keys", base_url))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({
            "key_pem": TEST_ED25519_KEY,
        }))
        .send()
        .await?;

    assert_eq!(res.status(), 200, "valid key upload should succeed");
    let body: serde_json::Value = res.json().await?;
    let key_id = body["key_id"].as_str().expect("should return key_id");
    assert!(!key_id.is_empty(), "key_id should not be empty");
    assert!(key_id.contains('-'), "key_id should be a UUID");

    println!("test_upload_valid_key: key_id = {}", key_id);
    Ok(())
}

#[tokio::test]
async fn test_upload_invalid_key() -> Result<()> {
    let base_url = std::env::var("BASE_URL").unwrap_or_else(|_| "https://oxmux.app".to_string());
    let client = reqwest::Client::new();
    let token = get_auth_token(&client, &base_url).await?;

    let res = client
        .post(format!("{}/api/ssh-keys", base_url))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({
            "key_pem": "not a valid key",
        }))
        .send()
        .await?;

    assert_eq!(res.status(), 400, "invalid key should be rejected");
    let body = res.text().await?;
    assert!(body.contains("invalid") || body.contains("unsupported"), "error should mention invalid format");
    // Should NOT contain the key content
    assert!(!body.contains("not a valid key"), "error should not echo back the key content");

    println!("test_upload_invalid_key: correctly rejected");
    Ok(())
}

#[tokio::test]
async fn test_upload_empty_key() -> Result<()> {
    let base_url = std::env::var("BASE_URL").unwrap_or_else(|_| "https://oxmux.app".to_string());
    let client = reqwest::Client::new();
    let token = get_auth_token(&client, &base_url).await?;

    let res = client
        .post(format!("{}/api/ssh-keys", base_url))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({
            "key_pem": "",
        }))
        .send()
        .await?;

    assert_eq!(res.status(), 400, "empty key should be rejected");

    println!("test_upload_empty_key: correctly rejected");
    Ok(())
}

#[tokio::test]
async fn test_upload_key_too_large() -> Result<()> {
    let base_url = std::env::var("BASE_URL").unwrap_or_else(|_| "https://oxmux.app".to_string());
    let client = reqwest::Client::new();
    let token = get_auth_token(&client, &base_url).await?;

    let large_key = "A".repeat(9000); // > 8KB limit
    let res = client
        .post(format!("{}/api/ssh-keys", base_url))
        .header("Authorization", format!("Bearer {}", token))
        .json(&serde_json::json!({
            "key_pem": large_key,
        }))
        .send()
        .await?;

    assert_eq!(res.status(), 413, "oversized key should be rejected");

    println!("test_upload_key_too_large: correctly rejected");
    Ok(())
}

#[tokio::test]
async fn test_upload_without_auth() -> Result<()> {
    let base_url = std::env::var("BASE_URL").unwrap_or_else(|_| "https://oxmux.app".to_string());
    let client = reqwest::Client::new();

    let res = client
        .post(format!("{}/api/ssh-keys", base_url))
        .json(&serde_json::json!({
            "key_pem": TEST_ED25519_KEY,
        }))
        .send()
        .await?;

    assert_eq!(res.status(), 401, "unauthenticated upload should be rejected");

    println!("test_upload_without_auth: correctly rejected");
    Ok(())
}

#[tokio::test]
async fn test_upload_with_invalid_token() -> Result<()> {
    let base_url = std::env::var("BASE_URL").unwrap_or_else(|_| "https://oxmux.app".to_string());
    let client = reqwest::Client::new();

    let res = client
        .post(format!("{}/api/ssh-keys", base_url))
        .header("Authorization", "Bearer invalid.token.here")
        .json(&serde_json::json!({
            "key_pem": TEST_ED25519_KEY,
        }))
        .send()
        .await?;

    assert_eq!(res.status(), 401, "invalid token should be rejected");

    println!("test_upload_with_invalid_token: correctly rejected");
    Ok(())
}
