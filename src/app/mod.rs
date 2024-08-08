pub mod serve;

pub use serve::serve;

use derive_more::Deref;
use std::{net::SocketAddr, sync::Arc};
use tokio::net::TcpListener;
use tracing::info;

use crate::{
    config::AppConfig, model::ModelManager, templ_manager::TemplateManager, EmailClient, Result,
};

// ###################################
// ->  Structs
// ###################################
pub struct App {
    pub app_state: AppState,
    pub listener: TcpListener,
}
impl App {
    pub fn new(app_state: AppState, listener: TcpListener) -> Self {
        App {
            app_state,
            listener,
        }
    }

    pub async fn build_from_config(config: &AppConfig) -> Result<Self> {
        let email_addr = config.email_config.valid_sender()?;

        let email_client = EmailClient::new(
            config.email_config.url.clone(),
            email_addr,
            config.email_config.auth_token.clone(),
            config.email_config.timeout(),
        )?;
        let tm = TemplateManager::init();

        let mm = ModelManager::init(config).await?;
        let base_url = config.net_config.base_url.clone();
        let app_state = AppState::new(mm, tm, email_client, base_url);

        let addr = SocketAddr::from((config.net_config.host, config.net_config.app_port));
        let listener = TcpListener::bind(addr).await?;
        let addr = listener.local_addr()?;
        info!("{:<20} - {}", "Listening on:", addr);

        let app = App::new(app_state, listener);
        Ok(app)
    }
}

pub struct InternalState {
    pub model_mgr: ModelManager,
    pub templ_mgr: TemplateManager,
    pub email_client: EmailClient,
    pub base_url: String,
}

/// Application state containing all global data.
/// It implements `Deref` to easily access the fields on `InternalState`
/// Contains an `Arc` so it is cheap to clone!
#[derive(Clone, Deref)]
pub struct AppState(Arc<InternalState>);

impl AppState {
    pub fn new(
        model_mgr: ModelManager,
        templ_mgr: TemplateManager,
        email_client: EmailClient,
        base_url: String,
    ) -> Self {
        AppState(Arc::new(InternalState {
            templ_mgr,
            model_mgr,
            email_client,
            base_url,
        }))
    }
}
