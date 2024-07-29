use std::{net::SocketAddr, sync::OnceLock};

use anyhow::Result;
use mailomat::{config::get_or_init_config, init_dbg_tracing, model::ModelManager, App};
use reqwest::Client;
use uuid::Uuid;
use wiremock::MockServer;

pub struct TestApp {
    pub http_client: Client,
    pub addr: SocketAddr,
    pub mm: ModelManager,
    pub email_server: MockServer,
}
impl TestApp {
    pub fn new(addr: SocketAddr, mm: ModelManager, email_server: MockServer) -> Self {
        TestApp {
            addr,
            mm,
            http_client: Client::new(),
            email_server,
        }
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
}

fn _init_test_subscriber() {
    static SUBSCRIBER: OnceLock<()> = OnceLock::new();
    SUBSCRIBER.get_or_init(|| {
        init_dbg_tracing();
    });
}

/// A helper function that tries to spawn a separate thread to serve our app
/// returning the *socket address* on which it is listening.
pub async fn spawn_test_app() -> Result<TestApp> {
    // _init_test_subscriber();

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
    let mm = app.app_state.mm.clone();

    tokio::spawn(mailomat::serve(app));

    let res = TestApp::new(addr, mm, email_server);
    Ok(res)
}
