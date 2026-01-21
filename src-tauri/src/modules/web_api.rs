use axum::{
    extract::State,
    http::StatusCode,
    routing::post,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::models::{Account, AppConfig, DeviceProfile, QuotaData};
use crate::modules;
use crate::proxy;
use crate::proxy::server::ProxyRuntime;
use reqwest::Client;
use std::time::Duration;
use tower_http::cors::{Any, CorsLayer};

#[derive(Clone)]
pub struct WebApiState {
    pub token_manager: Arc<proxy::TokenManager>,
    pub proxy_runtime: ProxyRuntime,
    pub monitor: Arc<proxy::monitor::ProxyMonitor>,
}

#[derive(Debug, Deserialize)]
struct InvokeRequest {
    cmd: String,
    #[serde(default)]
    args: Value,
}

#[derive(Serialize)]
struct InvokeResponse {
    ok: bool,
    data: Option<Value>,
    error: Option<String>,
}

fn ok(data: Value) -> Json<InvokeResponse> {
    Json(InvokeResponse {
        ok: true,
        data: Some(data),
        error: None,
    })
}

fn err(status: StatusCode, message: String) -> (StatusCode, Json<InvokeResponse>) {
    (
        status,
        Json(InvokeResponse {
            ok: false,
            data: None,
            error: Some(message),
        }),
    )
}

pub fn router(state: WebApiState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        .route("/invoke", post(invoke_handler))
        .layer(cors)
        .with_state(state)
}

async fn invoke_handler(
    State(state): State<WebApiState>,
    Json(payload): Json<InvokeRequest>,
) -> Result<Json<InvokeResponse>, (StatusCode, Json<InvokeResponse>)> {
    let cmd = payload.cmd.as_str();
    let args = payload.args;

    match cmd {
        "list_accounts" => {
            let accounts = modules::list_accounts().map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(accounts)))
        }
        "get_current_account" => {
            let account = modules::account::get_current_account()
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(account)))
        }
        "add_account" => {
            #[derive(Deserialize)]
            struct AddArgs {
                #[serde(default)]
                email: String,
                refreshToken: String,
            }
            let input: AddArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;

            let token_res = modules::oauth::refresh_access_token(&input.refreshToken)
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            let user_info = modules::oauth::get_user_info(&token_res.access_token)
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;

            let token = crate::models::TokenData::new(
                token_res.access_token,
                input.refreshToken,
                token_res.expires_in,
                Some(user_info.email.clone()),
                None,
                None,
            );

            let mut account = modules::upsert_account(
                user_info.email.clone(),
                user_info.get_display_name(),
                token,
            )
            .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;

            let _ = internal_refresh_account_quota(&mut account).await;
            let _ = state.token_manager.reload_account(&account.id).await;

            Ok(ok(json!(account)))
        }
        "delete_account" => {
            #[derive(Deserialize)]
            struct DeleteArgs {
                accountId: String,
            }
            let input: DeleteArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            modules::delete_account(&input.accountId)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            let _ = state.token_manager.reload_all_accounts().await;
            Ok(ok(json!(true)))
        }
        "delete_accounts" => {
            #[derive(Deserialize)]
            struct DeleteArgs {
                accountIds: Vec<String>,
            }
            let input: DeleteArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            modules::account::delete_accounts(&input.accountIds)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            let _ = state.token_manager.reload_all_accounts().await;
            Ok(ok(json!(true)))
        }
        "switch_account" => {
            #[derive(Deserialize)]
            struct SwitchArgs {
                accountId: String,
            }
            let input: SwitchArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            modules::switch_account(&input.accountId)
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            state.token_manager.clear_all_sessions();
            let _ = state.token_manager.reload_all_accounts().await;
            Ok(ok(json!(true)))
        }
        "fetch_account_quota" => {
            #[derive(Deserialize)]
            struct QuotaArgs {
                accountId: String,
            }
            let input: QuotaArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            let mut account = modules::load_account(&input.accountId)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            let quota = modules::account::fetch_quota_with_retry(&mut account)
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            modules::update_account_quota(&input.accountId, quota.clone())
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            let _ = state.token_manager.reload_account(&input.accountId).await;
            Ok(ok(json!(quota)))
        }
        "refresh_all_quotas" => {
            let stats = modules::account::refresh_all_quotas_logic()
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            let _ = state.token_manager.reload_all_accounts().await;
            Ok(ok(json!(stats)))
        }
        "import_v1_accounts" => {
            let accounts = modules::migration::import_from_v1()
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            for mut account in accounts.clone() {
                let _ = internal_refresh_account_quota(&mut account).await;
            }
            let _ = state.token_manager.reload_all_accounts().await;
            Ok(ok(json!(accounts)))
        }
        "import_from_db" => {
            let mut account = modules::migration::import_from_db()
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            let account_id = account.id.clone();
            modules::account::set_current_account_id(&account_id)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            let _ = internal_refresh_account_quota(&mut account).await;
            let _ = state.token_manager.reload_all_accounts().await;
            Ok(ok(json!(account)))
        }
        "import_custom_db" => {
            #[derive(Deserialize)]
            struct ImportArgs {
                path: String,
            }
            let input: ImportArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            let mut account = modules::migration::import_from_custom_db_path(input.path)
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            let account_id = account.id.clone();
            modules::account::set_current_account_id(&account_id)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            let _ = internal_refresh_account_quota(&mut account).await;
            let _ = state.token_manager.reload_all_accounts().await;
            Ok(ok(json!(account)))
        }
        "sync_account_from_db" => {
            let db_refresh_token = match modules::migration::get_refresh_token_from_db() {
                Ok(token) => token,
                Err(e) => {
                    modules::logger::log_info(&format!("自动同步跳过: {}", e));
                    return Ok(ok(json!(Option::<Account>::None)));
                }
            };

            let curr_account = modules::account::get_current_account()
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            if let Some(acc) = curr_account {
                if acc.token.refresh_token == db_refresh_token {
                    return Ok(ok(json!(Option::<Account>::None)));
                }
            }
            let mut account = modules::migration::import_from_db()
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            let account_id = account.id.clone();
            modules::account::set_current_account_id(&account_id)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            let _ = internal_refresh_account_quota(&mut account).await;
            let _ = state.token_manager.reload_all_accounts().await;
            Ok(ok(json!(Some(account))))
        }
        "reorder_accounts" => {
            #[derive(Deserialize)]
            struct ReorderArgs {
                accountIds: Vec<String>,
            }
            let input: ReorderArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            modules::account::reorder_accounts(&input.accountIds)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(true)))
        }
        "toggle_proxy_status" => {
            #[derive(Deserialize)]
            struct ToggleArgs {
                accountId: String,
                enable: bool,
                reason: Option<String>,
            }
            let input: ToggleArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            toggle_proxy_status(&input.accountId, input.enable, input.reason)
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            let _ = state.token_manager.reload_all_accounts().await;
            Ok(ok(json!(true)))
        }
        "get_device_profiles" => {
            #[derive(Deserialize)]
            struct DeviceArgs {
                accountId: String,
            }
            let input: DeviceArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            let profiles = modules::get_device_profiles(&input.accountId)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(profiles)))
        }
        "bind_device_profile" => {
            #[derive(Deserialize)]
            struct BindArgs {
                accountId: String,
                mode: String,
            }
            let input: BindArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            let profile = modules::bind_device_profile(&input.accountId, &input.mode)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(profile)))
        }
        "restore_original_device" => {
            let msg = modules::account::restore_original_device()
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(msg)))
        }
        "list_device_versions" => {
            #[derive(Deserialize)]
            struct DeviceArgs {
                accountId: String,
            }
            let input: DeviceArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            let res = modules::list_device_versions(&input.accountId)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(res)))
        }
        "restore_device_version" => {
            #[derive(Deserialize)]
            struct RestoreArgs {
                accountId: String,
                versionId: String,
            }
            let input: RestoreArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            let profile = modules::restore_device_version(&input.accountId, &input.versionId)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(profile)))
        }
        "delete_device_version" => {
            #[derive(Deserialize)]
            struct DeleteArgs {
                accountId: String,
                versionId: String,
            }
            let input: DeleteArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            modules::delete_device_version(&input.accountId, &input.versionId)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(true)))
        }
        "preview_generate_profile" => {
            let profile = modules::device::generate_profile();
            Ok(ok(json!(profile)))
        }
        "bind_device_profile_with_profile" => {
            #[derive(Deserialize)]
            struct BindArgs {
                accountId: String,
                profile: DeviceProfile,
            }
            let input: BindArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            let profile =
                modules::bind_device_profile_with_profile(&input.accountId, input.profile, Some("generated".to_string()))
                    .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(profile)))
        }
        "warm_up_all_accounts" => {
            let msg = modules::quota::warm_up_all_accounts()
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(msg)))
        }
        "warm_up_account" => {
            #[derive(Deserialize)]
            struct WarmArgs {
                accountId: String,
            }
            let input: WarmArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            let msg = modules::quota::warm_up_account(&input.accountId)
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(msg)))
        }
        "load_config" => {
            let config = modules::config::load_app_config()
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(config)))
        }
        "save_config" => {
            #[derive(Deserialize)]
            struct SaveArgs {
                config: AppConfig,
            }
            let input: SaveArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            modules::config::save_app_config(&input.config)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            apply_proxy_config(&state, &input.config.proxy).await;
            state
                .token_manager
                .update_sticky_config(input.config.proxy.scheduling.clone())
                .await;
            Ok(ok(json!(true)))
        }
        "get_proxy_status" => {
            let config = modules::config::load_app_config()
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            let active_accounts = state.token_manager.len();
            Ok(ok(json!({
                "running": config.proxy.enabled,
                "port": config.proxy.port,
                "base_url": format!("http://{}:{}", config.proxy.get_bind_address(), config.proxy.port),
                "active_accounts": active_accounts
            })))
        }
        "generate_api_key" => {
            let key = format!("sk-{}", uuid::Uuid::new_v4().simple());
            Ok(ok(json!(key)))
        }
        "start_proxy_service" => {
            let mut config = modules::config::load_app_config()
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            config.proxy.enabled = true;
            modules::config::save_app_config(&config)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!({
                "running": true,
                "port": config.proxy.port,
                "base_url": format!("http://{}:{}", config.proxy.get_bind_address(), config.proxy.port),
                "active_accounts": state.token_manager.len()
            })))
        }
        "stop_proxy_service" => {
            let mut config = modules::config::load_app_config()
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            config.proxy.enabled = false;
            modules::config::save_app_config(&config)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!({
                "running": false,
                "port": config.proxy.port,
                "base_url": format!("http://{}:{}", config.proxy.get_bind_address(), config.proxy.port),
                "active_accounts": state.token_manager.len()
            })))
        }
        "update_model_mapping" => {
            #[derive(Deserialize)]
            struct UpdateArgs {
                config: proxy::ProxyConfig,
            }
            let input: UpdateArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            {
                let mut mapping = state.proxy_runtime.custom_mapping.write().await;
                *mapping = input.config.custom_mapping.clone();
            }
            let mut config = modules::config::load_app_config()
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            config.proxy.custom_mapping = input.config.custom_mapping;
            modules::config::save_app_config(&config)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(true)))
        }
        "fetch_zai_models" => {
            #[derive(Deserialize)]
            struct ZaiArgs {
                zai: Option<proxy::ZaiConfig>,
                upstreamProxy: Option<proxy::config::UpstreamProxyConfig>,
                requestTimeout: Option<u64>,
                base_url: Option<String>,
            }
            let input: ZaiArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            let config = modules::config::load_app_config()
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            let mut zai = input.zai.unwrap_or_else(|| config.proxy.zai.clone());
            if let Some(base_url) = input.base_url {
                zai.base_url = base_url;
            }
            let upstream_proxy = input
                .upstreamProxy
                .unwrap_or_else(|| config.proxy.upstream_proxy.clone());
            let request_timeout = input.requestTimeout.unwrap_or(config.proxy.request_timeout);
            let models = fetch_zai_models(zai, upstream_proxy, request_timeout)
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(models)))
        }
        "clear_proxy_session_bindings" => {
            state.token_manager.clear_all_sessions();
            Ok(ok(json!(true)))
        }
        "set_preferred_account" => {
            #[derive(Deserialize)]
            struct PreferredArgs {
                accountId: Option<String>,
            }
            let input: PreferredArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            let cleaned = input.accountId.filter(|v| !v.trim().is_empty());
            state.token_manager.set_preferred_account(cleaned).await;
            Ok(ok(json!(true)))
        }
        "get_preferred_account" => {
            let preferred = state.token_manager.get_preferred_account().await;
            Ok(ok(json!(preferred)))
        }
        "get_proxy_stats" => {
            let stats = modules::proxy_db::get_stats()
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(stats)))
        }
        "get_proxy_logs_count_filtered" => {
            #[derive(Deserialize)]
            struct LogsArgs {
                filter: String,
                errorsOnly: bool,
            }
            let input: LogsArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            let count = modules::proxy_db::get_logs_count_filtered(&input.filter, input.errorsOnly)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(count)))
        }
        "get_proxy_logs_filtered" => {
            #[derive(Deserialize)]
            struct LogsArgs {
                filter: String,
                errorsOnly: bool,
                limit: usize,
                offset: usize,
            }
            let input: LogsArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            let logs = modules::proxy_db::get_logs_filtered(&input.filter, input.errorsOnly, input.limit, input.offset)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(logs)))
        }
        "set_proxy_monitor_enabled" => {
            #[derive(Deserialize)]
            struct MonitorArgs {
                enabled: bool,
            }
            let input: MonitorArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            state.monitor.set_enabled(input.enabled);
            Ok(ok(json!(true)))
        }
        "clear_proxy_logs" => {
            state.monitor.clear().await;
            Ok(ok(json!(true)))
        }
        "execute_cli_sync" => {
            #[derive(Deserialize)]
            struct CliArgs {
                appType: proxy::cli_sync::CliApp,
                proxyUrl: String,
                apiKey: String,
            }
            let input: CliArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            proxy::cli_sync::execute_cli_sync(input.appType, input.proxyUrl, input.apiKey)
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(true)))
        }
        "execute_cli_restore" => {
            #[derive(Deserialize)]
            struct CliArgs {
                appType: proxy::cli_sync::CliApp,
            }
            let input: CliArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            proxy::cli_sync::execute_cli_restore(input.appType)
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(true)))
        }
        "get_cli_sync_status" => {
            #[derive(Deserialize)]
            struct CliArgs {
                appType: proxy::cli_sync::CliApp,
                proxyUrl: String,
            }
            let input: CliArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            let status = proxy::cli_sync::get_cli_sync_status(input.appType, input.proxyUrl)
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(status)))
        }
        "get_cli_config_content" => {
            #[derive(Deserialize)]
            struct CliArgs {
                appType: proxy::cli_sync::CliApp,
                fileName: Option<String>,
            }
            let input: CliArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            let content = proxy::cli_sync::get_cli_config_content(input.appType, input.fileName)
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(content)))
        }
        "get_token_stats_hourly" => {
            let hours = args.get("hours").and_then(|v| v.as_i64()).unwrap_or(24);
            let data = modules::token_stats::get_hourly_stats(hours)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(data)))
        }
        "get_token_stats_daily" => {
            let days = args.get("days").and_then(|v| v.as_i64()).unwrap_or(7);
            let data = modules::token_stats::get_daily_stats(days)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(data)))
        }
        "get_token_stats_weekly" => {
            let weeks = args.get("weeks").and_then(|v| v.as_i64()).unwrap_or(4);
            let data = modules::token_stats::get_weekly_stats(weeks)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(data)))
        }
        "get_token_stats_by_account" => {
            let hours = args.get("hours").and_then(|v| v.as_i64()).unwrap_or(24);
            let data = modules::token_stats::get_account_stats(hours)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(data)))
        }
        "get_token_stats_by_model" => {
            let hours = args.get("hours").and_then(|v| v.as_i64()).unwrap_or(24);
            let data = modules::token_stats::get_model_stats(hours)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(data)))
        }
        "get_token_stats_summary" => {
            let hours = args.get("hours").and_then(|v| v.as_i64()).unwrap_or(24);
            let data = modules::token_stats::get_summary_stats(hours)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(data)))
        }
        "get_token_stats_model_trend_hourly" => {
            let hours = args.get("hours").and_then(|v| v.as_i64()).unwrap_or(24);
            let data = modules::token_stats::get_model_trend_hourly(hours)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(data)))
        }
        "get_token_stats_model_trend_daily" => {
            let days = args.get("days").and_then(|v| v.as_i64()).unwrap_or(7);
            let data = modules::token_stats::get_model_trend_daily(days)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(data)))
        }
        "get_token_stats_account_trend_hourly" => {
            let hours = args.get("hours").and_then(|v| v.as_i64()).unwrap_or(24);
            let data = modules::token_stats::get_account_trend_hourly(hours)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(data)))
        }
        "get_token_stats_account_trend_daily" => {
            let days = args.get("days").and_then(|v| v.as_i64()).unwrap_or(7);
            let data = modules::token_stats::get_account_trend_daily(days)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(data)))
        }
        "check_for_updates" => {
            let info = modules::update_checker::check_for_updates()
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(info)))
        }
        "should_check_updates" => {
            let settings = modules::update_checker::load_update_settings()
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            let should = modules::update_checker::should_check_for_updates(&settings);
            Ok(ok(json!(should)))
        }
        "update_last_check_time" => {
            modules::update_checker::update_last_check_time()
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(true)))
        }
        "get_update_settings" => {
            let settings = modules::update_checker::load_update_settings()
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(settings)))
        }
        "save_update_settings" => {
            #[derive(Deserialize)]
            struct UpdateArgs {
                settings: modules::update_checker::UpdateSettings,
            }
            let input: UpdateArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            modules::update_checker::save_update_settings(&input.settings)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(true)))
        }
        "get_http_api_settings" => {
            let settings = modules::http_api::load_settings()
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(settings)))
        }
        "save_http_api_settings" => {
            #[derive(Deserialize)]
            struct SettingsArgs {
                settings: modules::http_api::HttpApiSettings,
            }
            let input: SettingsArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            modules::http_api::save_settings(&input.settings)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(true)))
        }
        "toggle_auto_launch" => {
            #[derive(Deserialize)]
            struct AutoArgs {
                enable: bool,
            }
            let input: AutoArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            let mut config = modules::config::load_app_config()
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            config.auto_launch = input.enable;
            modules::config::save_app_config(&config)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(true)))
        }
        "open_data_folder" => {
            let dir = modules::account::get_data_dir()
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(dir.to_string_lossy().to_string())))
        }
        "get_data_dir_path" => {
            let dir = modules::account::get_data_dir()
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(dir.to_string_lossy().to_string())))
        }
        "open_device_folder" => {
            let dir = modules::device::get_storage_dir()
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(dir.to_string_lossy().to_string())))
        }
        "get_antigravity_path" => {
            match modules::process::get_antigravity_executable_path() {
                Some(path) => Ok(ok(json!(path.to_string_lossy().to_string()))),
                None => Err(err(StatusCode::NOT_FOUND, "未找到 Antigravity 安装路径".to_string())),
            }
        }
        "get_antigravity_args" => {
            match modules::process::get_args_from_running_process() {
                Some(args) => Ok(ok(json!(args))),
                None => Err(err(StatusCode::NOT_FOUND, "未找到正在运行的 Antigravity 进程".to_string())),
            }
        }
        "start_oauth_login" => {
            #[derive(Deserialize)]
            struct OAuthArgs {
                redirectUri: String,
            }
            let input: OAuthArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            let url = modules::oauth::get_auth_url(&input.redirectUri);
            Ok(ok(json!({ "auth_url": url })))
        }
        "complete_oauth_login" => {
            #[derive(Deserialize)]
            struct OAuthArgs {
                code: String,
                redirectUri: String,
            }
            let input: OAuthArgs = serde_json::from_value(args)
                .map_err(|e| err(StatusCode::BAD_REQUEST, e.to_string()))?;
            let token_res = modules::oauth::exchange_code(&input.code, &input.redirectUri)
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            let refresh_token = match token_res.refresh_token {
                Some(token) => token,
                None => {
                    return Err(err(
                        StatusCode::BAD_REQUEST,
                        "未获取到 Refresh Token。请先在 Google 账号授权页撤销访问后重试，或使用 Refresh Token 手动添加账号。".to_string(),
                    ));
                }
            };
            let user_info = modules::oauth::get_user_info(&token_res.access_token)
                .await
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            let project_id = crate::proxy::project_resolver::fetch_project_id(&token_res.access_token)
                .await
                .ok();
            let token = crate::models::TokenData::new(
                token_res.access_token,
                refresh_token,
                token_res.expires_in,
                Some(user_info.email.clone()),
                project_id,
                None,
            );
            let mut account = modules::upsert_account(
                user_info.email.clone(),
                user_info.get_display_name(),
                token,
            )
            .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            let _ = internal_refresh_account_quota(&mut account).await;
            let _ = state.token_manager.reload_all_accounts().await;
            Ok(ok(json!(account)))
        }
        "cancel_oauth_login" => Ok(ok(json!(true))),
        "clear_log_cache" => {
            modules::logger::clear_logs()
                .map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
            Ok(ok(json!(true)))
        }
        "save_text_file" | "read_text_file" => {
            Err(err(StatusCode::NOT_IMPLEMENTED, "File operations are handled in the browser in web mode.".to_string()))
        }
        _ => Err(err(
            StatusCode::NOT_FOUND,
            format!("Unknown command: {}", payload.cmd),
        )),
    }
}

async fn internal_refresh_account_quota(account: &mut Account) -> Result<QuotaData, String> {
    match modules::account::fetch_quota_with_retry(account).await {
        Ok(quota) => {
            let _ = modules::update_account_quota(&account.id, quota.clone());
            Ok(quota)
        }
        Err(e) => Err(e.to_string()),
    }
}

async fn apply_proxy_config(state: &WebApiState, config: &proxy::ProxyConfig) {
    {
        let mut mapping = state.proxy_runtime.custom_mapping.write().await;
        *mapping = config.custom_mapping.clone();
    }
    {
        let mut proxy_state = state.proxy_runtime.proxy_state.write().await;
        *proxy_state = config.upstream_proxy.clone();
    }
    {
        let mut security = state.proxy_runtime.security_state.write().await;
        *security = proxy::ProxySecurityConfig::from_proxy_config(config);
    }
    {
        let mut zai = state.proxy_runtime.zai_state.write().await;
        *zai = config.zai.clone();
    }
    {
        let mut experimental = state.proxy_runtime.experimental.write().await;
        *experimental = config.experimental.clone();
    }
}

async fn toggle_proxy_status(account_id: &str, enable: bool, reason: Option<String>) -> Result<(), String> {
    let data_dir = modules::account::get_data_dir()?;
    let account_path = data_dir.join("accounts").join(format!("{}.json", account_id));

    if !account_path.exists() {
        return Err(format!("账号文件不存在: {}", account_id));
    }

    let content = std::fs::read_to_string(&account_path)
        .map_err(|e| format!("读取账号文件失败: {}", e))?;
    let mut account_json: Value = serde_json::from_str(&content)
        .map_err(|e| format!("解析账号文件失败: {}", e))?;

    if enable {
        account_json["proxy_disabled"] = Value::Bool(false);
        account_json["proxy_disabled_reason"] = Value::Null;
        account_json["proxy_disabled_at"] = Value::Null;
    } else {
        let now = chrono::Utc::now().timestamp();
        account_json["proxy_disabled"] = Value::Bool(true);
        account_json["proxy_disabled_at"] = Value::Number(now.into());
        account_json["proxy_disabled_reason"] = Value::String(
            reason.unwrap_or_else(|| "用户手动禁用".to_string())
        );
    }

    std::fs::write(&account_path, serde_json::to_string_pretty(&account_json).unwrap())
        .map_err(|e| format!("写入账号文件失败: {}", e))?;
    Ok(())
}

fn join_base_url(base: &str, path: &str) -> String {
    let base = base.trim_end_matches('/');
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };
    format!("{}{}", base, path)
}

fn extract_model_ids(value: &Value) -> Vec<String> {
    let mut out = Vec::new();

    fn push_from_item(out: &mut Vec<String>, item: &Value) {
        match item {
            Value::String(s) => out.push(s.to_string()),
            Value::Object(map) => {
                if let Some(id) = map.get("id").and_then(|v| v.as_str()) {
                    out.push(id.to_string());
                } else if let Some(name) = map.get("name").and_then(|v| v.as_str()) {
                    out.push(name.to_string());
                }
            }
            _ => {}
        }
    }

    match value {
        Value::Array(arr) => {
            for item in arr {
                push_from_item(&mut out, item);
            }
        }
        Value::Object(map) => {
            if let Some(data) = map.get("data") {
                if let Value::Array(arr) = data {
                    for item in arr {
                        push_from_item(&mut out, item);
                    }
                }
            }
            if let Some(models) = map.get("models") {
                match models {
                    Value::Array(arr) => {
                        for item in arr {
                            push_from_item(&mut out, item);
                        }
                    }
                    other => push_from_item(&mut out, other),
                }
            }
        }
        _ => {}
    }

    out
}

async fn fetch_zai_models(
    zai: proxy::ZaiConfig,
    upstream_proxy: proxy::config::UpstreamProxyConfig,
    request_timeout: u64,
) -> Result<Vec<String>, String> {
    if zai.base_url.trim().is_empty() {
        return Err("z.ai base_url is empty".to_string());
    }
    if zai.api_key.trim().is_empty() {
        return Err("z.ai api_key is not set".to_string());
    }

    let url = join_base_url(&zai.base_url, "/v1/models");
    let mut builder = Client::builder().timeout(Duration::from_secs(request_timeout.max(5)));
    if upstream_proxy.enabled && !upstream_proxy.url.is_empty() {
        let proxy = reqwest::Proxy::all(&upstream_proxy.url)
            .map_err(|e| format!("Invalid upstream proxy url: {}", e))?;
        builder = builder.proxy(proxy);
    }
    let client = builder
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))?;

    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", zai.api_key))
        .header("x-api-key", zai.api_key)
        .header("anthropic-version", "2023-06-01")
        .header("accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Upstream request failed: {}", e))?;

    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;
    if !status.is_success() {
        let preview = if text.len() > 4000 { &text[..4000] } else { &text };
        return Err(format!("Upstream returned {}: {}", status, preview));
    }

    let json: Value = serde_json::from_str(&text)
        .map_err(|e| format!("Invalid JSON response: {}", e))?;
    let mut models = extract_model_ids(&json);
    models.retain(|s| !s.trim().is_empty());
    models.sort();
    models.dedup();
    Ok(models)
}
