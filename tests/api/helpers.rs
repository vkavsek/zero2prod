use std::{net::SocketAddr, sync::OnceLock};

use anyhow::{Context, Result};
use fake::Fake;
use linkify::LinkKind;
use mailomat::{
    config::get_or_init_config,
    model::ModelManager,
    utils::b64_encode,
    web::data::{DeserSubscriber, ValidSubscriber},
    App,
};
use reqwest::Client;
use serde_json::{json, Value};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

fn init_test_subscriber() {
    static SUBSCRIBER: OnceLock<()> = OnceLock::new();
    SUBSCRIBER.get_or_init(|| {
        tracing_subscriber::fmt()
            .without_time()
            .with_target(false)
            .with_env_filter(EnvFilter::from_env("TEST_LOG"))
            .compact()
            .init();
    });
}

pub struct ConfirmationLinks {
    pub html: reqwest::Url,
    pub plain_text: reqwest::Url,
}

pub struct TestApp {
    pub http_client: Client,
    pub addr: SocketAddr,
    pub mm: ModelManager,
    pub email_server: MockServer,
}
impl TestApp {
    /// A helper function that tries to spawn a separate thread to serve our app
    /// returning the *socket address* on which it is listening.
    pub async fn spawn() -> Result<Self> {
        init_test_subscriber();

        // A mock server to stand-in for Postmark API
        let email_server = MockServer::start().await;

        let config = {
            let mut c = get_or_init_config().to_owned();
            // A new name for each test
            c.db_config.db_name = Uuid::new_v4().to_string();
            // Trying to bind port 0 will trigger an OS scan for an available port
            // which will then be bound to the application.
            c.net_config.app_port = 0;
            c.email_config.url = email_server.uri();
            c
        };

        // Create and migrate the test DB
        ModelManager::configure_for_test(&config).await?;

        let app = App::build_from_config(&config).await?;

        let addr = app.listener.local_addr()?;
        let mm = app.app_state.model_mgr.clone();
        let http_client = Client::new();

        tokio::spawn(mailomat::serve(app));

        Ok(TestApp {
            http_client,
            addr,
            mm,
            email_server,
        })
    }

    pub async fn post_subscriptions(&self, body: &serde_json::Value) -> Result<reqwest::Response> {
        let res = self
            .http_client
            .post(format!("http://{}/api/subscribe", self.addr))
            .json(body)
            .send()
            .await?;

        Ok(res)
    }

    pub async fn post_unauthorized_api_news(&self) -> Result<reqwest::Response> {
        // A sketch of the current newsletter payload structure.
        let newsletter_req_body = json!({
            "title": "Newsletter title",
            "content": {
                "text": "Newsletter body as plain text",
                "html": "<p>Newsletter body as HTML</p>",
            }
        });

        let res = self
            .http_client
            .post(&format!("http://{}/api/news", &self.addr))
            .json(&newsletter_req_body)
            .send()
            .await?;

        Ok(res)
    }

    pub async fn post_api_news(&self) -> Result<reqwest::Response> {
        // A sketch of the current newsletter payload structure.
        let newsletter_req_body = json!({
            "title": "Newsletter title",
            "content": {
                "text": "Newsletter body as plain text",
                "html": "<p>Newsletter body as HTML</p>",
            }
        });

        let creds = "admin:password";
        let b64_enc = b64_encode(creds);

        let res = self
            .http_client
            .post(&format!("http://{}/api/news", &self.addr))
            .header(reqwest::header::AUTHORIZATION, format!("Basic {b64_enc}"))
            .json(&newsletter_req_body)
            .send()
            .await?;

        Ok(res)
    }

    /// Extract confirmation links embedded in the request to the email API.
    pub fn get_confirmation_links(
        &self,
        email_req: &wiremock::Request,
    ) -> Result<ConfirmationLinks> {
        let body: Value = serde_json::from_slice(&email_req.body)?;

        let get_link = |s: &str| {
            let links: Vec<_> = linkify::LinkFinder::new()
                .links(s)
                .filter(|l| l.kind() == &LinkKind::Url)
                .collect();
            assert_eq!(links.len(), 1);

            let raw_link = links[0].as_str().to_owned();
            let mut confirm_link = reqwest::Url::parse(&raw_link)?;
            // Check that we don''s on the web.
            assert_eq!(confirm_link.host_str(), Some("127.0.0.1"));
            confirm_link.set_port(Some(self.addr.port())).unwrap();
            Ok::<reqwest::Url, anyhow::Error>(confirm_link)
        };

        let html = get_link(body["HtmlBody"].as_str().context("No link in HtmlBody")?)?;
        let plain_text = get_link(body["TextBody"].as_str().context("No link in TextBody")?)?;
        Ok(ConfirmationLinks { html, plain_text })
    }

    /// Create new subscriber with: NAME - *John Doe*, EMAIL - *john.doe@example.com*
    /// Returns confirmation links required to confirm this subscriber and the subscriber's info.
    pub async fn create_unconfirmed_subscriber(
        &self,
    ) -> Result<(ConfirmationLinks, ValidSubscriber)> {
        let name: String = fake::faker::name::en::Name().fake();
        let email_provider: String = fake::faker::internet::en::FreeEmailProvider().fake();
        let email = name.to_lowercase().replace(" ", "_") + "@" + &email_provider;

        let body = json!({
            "name": name,
            "email": email
        });
        let valid_sub = ValidSubscriber::try_from(DeserSubscriber::new(name, email))?;

        let _mock_guard = Mock::given(path("/email"))
            .and(method("POST"))
            .respond_with(ResponseTemplate::new(200))
            .named("Create unconfirmed subscriber")
            .expect(1)
            .mount_as_scoped(&self.email_server)
            .await;

        self.post_subscriptions(&body).await?.error_for_status()?;
        let email_req = &self
            .email_server
            .received_requests()
            .await
            .expect("Requests should be received")
            .pop()
            .expect("1 request is expected");
        let links = self.get_confirmation_links(email_req)?;

        Ok((links, valid_sub))
    }

    /// Create new subscriber with: NAME - *John Doe*, EMAIL - *john.doe@example.com*
    /// and confirm it. Returns the info of the subscriber that was just added and confirmed.
    pub async fn create_confirmed_subscriber(&self) -> Result<ValidSubscriber> {
        let (links, subscriber) = self.create_unconfirmed_subscriber().await?;
        self.http_client
            .get(links.html)
            .send()
            .await?
            .error_for_status()?;

        Ok(subscriber)
    }
}
