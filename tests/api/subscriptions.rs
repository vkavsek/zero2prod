use anyhow::Result;
use reqwest::StatusCode;
use serde_json::json;
use serial_test::serial;
use wiremock::{
    matchers::{method, path},
    Mock, ResponseTemplate,
};

use crate::helpers::{ConfirmationLinks, TestApp};

#[serial]
#[tokio::test]
async fn api_subscribe_returns_200_for_valid_json() -> Result<()> {
    let app = TestApp::spawn().await?;

    let json_request = json!({
        "name": "John Doe",
        "email": "john.doe@example.com"
    });

    // Setup the mock server
    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&app.email_server)
        .await;

    let res = app.post_subscriptions(&json_request).await?;

    assert_eq!(
        res.status(),
        StatusCode::OK,
        "Wrong response StatusCode: {}",
        res.status()
    );

    Ok(())
}

#[serial]
#[tokio::test]
async fn api_subscribe_persists_the_new_subscriber() -> Result<()> {
    let app = TestApp::spawn().await?;

    let json_request = json!({
        "name": "John Doe",
        "email": "john.doe@example.com"
    });

    // Setup the mock server
    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&app.email_server)
        .await;

    app.post_subscriptions(&json_request).await?;

    let (email, name, status): (String, String, String) =
        sqlx::query_as("SELECT email, name, status FROM subscriptions")
            .fetch_one(app.mm.db())
            .await?;

    assert_eq!(email, "john.doe@example.com");
    assert_eq!(name, "John Doe");
    assert_eq!(status, "pending_confirmation");

    Ok(())
}

#[serial]
#[tokio::test]
async fn api_subscribe_unprocessable_entity() -> Result<()> {
    let app = TestApp::spawn().await?;

    let tests = [
        (
            json!({
                "name": "John Doe",
            }),
            "Missing email",
        ),
        (
            json!({
                "name": null,
                "email": "jd@example.com",
            }),
            "Null name",
        ),
        (json!({}), "Empty json"),
    ];

    for (json_request, params) in tests {
        let res = app.post_subscriptions(&json_request).await?;
        assert_eq!(
            res.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "Wrong response: ({}), Expected: ({}); for request with: {params}",
            res.status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    Ok(())
}

#[serial]
#[tokio::test]
async fn api_subscribe_returns_a_400_when_fields_are_present_but_invalid() -> Result<()> {
    let app = TestApp::spawn().await?;

    let cases = vec![
        (
            json!({
                "name": "",
                "email": "jd@example.com",
            }),
            "Empty name",
        ),
        (
            json!({
                "name": "John Doe",
                "email": "",
            }),
            "Empty email",
        ),
        (
            json!({
                "name": "John Doe",
                "email": "not an email",
            }),
            "Invalid email",
        ),
    ];

    for (body, description) in cases {
        let response = app.post_subscriptions(&body).await?;
        assert_eq!(
            400,
            response.status().as_u16(),
            "The API did not return a 400 BAD REQUEST the payload was {}.",
            description
        );
    }

    Ok(())
}

#[serial]
#[tokio::test]
async fn api_subscribe_duplicated_subscription_still_returns_200() -> Result<()> {
    let app = TestApp::spawn().await?;
    let body = json!({
        "name": "Ursula",
        "email": "le_guin@gmail.com",
    });

    // Setup the mock server
    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&app.email_server)
        .await;

    for (i, body) in std::iter::repeat(body).take(2).enumerate() {
        let res = app.post_subscriptions(&body).await?;
        assert_eq!(res.status(), StatusCode::OK, "failed in iteration {i}");
    }

    Ok(())
}

#[serial]
#[tokio::test]
async fn api_subscribe_sends_a_confirmation_email_for_valid_data() -> Result<()> {
    let app = TestApp::spawn().await?;
    let body = json!({
        "name": "Ursula",
        "email": "le_guin@gmail.com",
    });

    // Setup the mock server
    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&app.email_server)
        .await;

    let res = app.post_subscriptions(&body).await?;
    assert_eq!(res.status(), StatusCode::OK);

    Ok(())
}

#[serial]
#[tokio::test]
async fn api_subscribe_sends_a_confirmation_email_with_a_link() -> Result<()> {
    let app = TestApp::spawn().await?;
    let body = json!({
        "name": "Ursula",
        "email": "le_guin@gmail.com",
    });

    // Setup the mock server
    Mock::given(path("/email"))
        .and(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&app.email_server)
        .await;

    app.post_subscriptions(&body).await?;

    // Get the first intercepted request
    let email_req = &app.email_server.received_requests().await.unwrap()[0];
    let ConfirmationLinks { html, plain_text } = app.get_confirmation_links(email_req)?;

    assert_eq!(html, plain_text);

    Ok(())
}
