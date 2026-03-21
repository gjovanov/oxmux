//! Auth integration tests — JWT, password hashing, token validation.

use anyhow::Result;

#[tokio::test]
async fn test_register_and_login() -> Result<()> {
    let base_url = std::env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:8080".to_string());
    let client = reqwest::Client::new();

    let username = format!("test_auth_{}", uuid::Uuid::new_v4().to_string().split('-').next().unwrap());
    let password = "test_password_123";

    // Register
    let res = client
        .post(format!("{}/api/auth/register", base_url))
        .json(&serde_json::json!({
            "username": username,
            "password": password,
        }))
        .send()
        .await?;
    assert_eq!(res.status(), 200, "register should succeed");

    let body: serde_json::Value = res.json().await?;
    let token = body.get("token").and_then(|v| v.as_str()).expect("should have token");
    let user_id = body.get("user").and_then(|u| u.get("id")).and_then(|v| v.as_str()).expect("should have user id");
    assert!(!token.is_empty());
    assert!(!user_id.is_empty());
    println!("Registered user: {} ({})", username, user_id);

    // Register same username should fail
    let res = client
        .post(format!("{}/api/auth/register", base_url))
        .json(&serde_json::json!({
            "username": username,
            "password": "different",
        }))
        .send()
        .await?;
    assert_eq!(res.status(), 409, "duplicate registration should fail");

    // Login with correct password
    let res = client
        .post(format!("{}/api/auth/login", base_url))
        .json(&serde_json::json!({
            "username": username,
            "password": password,
        }))
        .send()
        .await?;
    assert_eq!(res.status(), 200, "login should succeed");

    let body: serde_json::Value = res.json().await?;
    let login_token = body.get("token").and_then(|v| v.as_str()).expect("should have token");
    assert!(!login_token.is_empty());

    // Login with wrong password
    let res = client
        .post(format!("{}/api/auth/login", base_url))
        .json(&serde_json::json!({
            "username": username,
            "password": "wrong_password",
        }))
        .send()
        .await?;
    assert_eq!(res.status(), 401, "wrong password should fail");

    // Validate token via /api/auth/me
    let res = client
        .get(format!("{}/api/auth/me?token={}", base_url, login_token))
        .send()
        .await?;
    assert_eq!(res.status(), 200, "me should succeed");

    let body: serde_json::Value = res.json().await?;
    assert_eq!(body.get("username").and_then(|v| v.as_str()), Some(username.as_str()));

    // Invalid token
    let res = client
        .get(format!("{}/api/auth/me?token=invalid.token.here", base_url))
        .send()
        .await?;
    assert_eq!(res.status(), 401, "invalid token should fail");

    println!("Auth tests passed");
    Ok(())
}
