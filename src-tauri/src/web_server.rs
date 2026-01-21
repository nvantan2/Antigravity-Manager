use axum::Router;
use std::sync::Arc;
use tracing::{error, info, warn};

use crate::models::AppConfig;
use crate::modules;
use crate::proxy;

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    info!("Shutdown signal received");
}

pub async fn run() -> Result<(), String> {
    modules::logger::init_logger();

    if let Err(e) = modules::token_stats::init_db() {
        error!("Failed to initialize token stats database: {}", e);
    }

    let app_config = match modules::config::load_app_config() {
        Ok(config) => config,
        Err(err) => {
            warn!("Failed to load app config, using defaults: {}", err);
            AppConfig::new()
        }
    };

    let proxy_config = app_config.proxy;
    let app_data_dir = modules::account::get_data_dir()?;
    let _ = modules::account::get_accounts_dir()?;

    let token_manager = Arc::new(proxy::TokenManager::new(app_data_dir));
    token_manager.start_auto_cleanup();
    token_manager
        .update_sticky_config(proxy_config.scheduling.clone())
        .await;

    let active_accounts = token_manager
        .load_accounts()
        .await
        .map_err(|e| format!("加载账号失败: {}", e))?;

    let zai_enabled = proxy_config.zai.enabled
        && !matches!(proxy_config.zai.dispatch_mode, proxy::ZaiDispatchMode::Off);
    if active_accounts == 0 && !zai_enabled {
        warn!("No active accounts found; proxy requests may fail until accounts are added");
    }

    let monitor = Arc::new(proxy::monitor::ProxyMonitor::new(1000));
    monitor.set_enabled(proxy_config.enable_logging);

    let (proxy_router, runtime) = proxy::server::build_router(
        token_manager.clone(),
        proxy_config.custom_mapping.clone(),
        proxy_config.request_timeout,
        proxy_config.upstream_proxy.clone(),
        proxy::ProxySecurityConfig::from_proxy_config(&proxy_config),
        proxy_config.zai.clone(),
        monitor.clone(),
        proxy_config.experimental.clone(),
    );

    let web_api_router = modules::web_api::router(modules::web_api::WebApiState {
        token_manager: token_manager.clone(),
        proxy_runtime: runtime,
        monitor: monitor.clone(),
    });
    let app = Router::new()
        .merge(proxy_router)
        .nest("/api", web_api_router);

    let host = proxy_config.get_bind_address().to_string();
    let port = proxy_config.port;
    let addr = format!("{}:{}", host, port);
    info!("Web server listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("failed_to_bind_port: {}", e))?;

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| format!("failed_to_run_server: {}", e))?;

    Ok(())
}
