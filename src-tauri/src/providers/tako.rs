use serde::Serialize;
use serde_json::{json, Value};

const TAKO_IDENTITY_URL: &str = "https://tako.shiroha.tech/apiStats/api/verify-identity";
const TAKO_API_STATS_BASE: &str = "https://tako.shiroha.tech/apiStats/api";
const TAKO_MODELS_URL: &str = "https://tako.shiroha.tech/v1/models";

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TakoLoginResult {
    pub ok: bool,
    pub name: Option<String>,
    pub plan: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TakoIdentity {
    pub logged_in: bool,
    pub name: Option<String>,
    pub plan: Option<String>,
    pub offline: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TakoUsageWindow {
    pub used: f64,
    pub limit: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TakoUsage {
    pub ok: bool,
    pub window: TakoUsageWindow,
    pub daily: TakoUsageWindow,
    pub weekly: TakoUsageWindow,
    pub plan_name: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TakoModel {
    pub id: String,
    pub name: String,
    pub provider: String,
    pub clients: Vec<String>,
}

pub struct TakoService;

impl TakoService {
    pub async fn login(api_key: String) -> Result<TakoLoginResult, String> {
        verify_tako_key(api_key).await
    }

    pub async fn apply_key(api_key: String) -> Result<TakoLoginResult, String> {
        verify_tako_key(api_key).await
    }

    pub async fn current_identity(api_key: Option<String>) -> Result<TakoIdentity, String> {
        let Some(api_key) = api_key
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
        else {
            return Ok(TakoIdentity::default());
        };

        match verify_tako_key(api_key).await {
            Ok(result) if result.ok => Ok(TakoIdentity {
                logged_in: true,
                name: result.name,
                plan: result.plan,
                offline: false,
            }),
            Ok(_) => Ok(TakoIdentity::default()),
            Err(_) => Ok(TakoIdentity {
                logged_in: true,
                offline: true,
                ..Default::default()
            }),
        }
    }

    pub async fn logout() -> Result<bool, String> {
        Ok(true)
    }

    pub async fn usage(api_key: String) -> Result<TakoUsage, String> {
        fetch_tako_usage(api_key).await
    }

    pub async fn list_models(api_key: String) -> Result<Vec<TakoModel>, String> {
        list_tako_models(api_key).await
    }
}

#[tauri::command]
pub async fn tako_login(api_key: String) -> Result<TakoLoginResult, String> {
    TakoService::login(api_key).await
}

#[tauri::command]
pub async fn tako_apply_key(api_key: String) -> Result<TakoLoginResult, String> {
    TakoService::apply_key(api_key).await
}

#[tauri::command]
pub async fn tako_current_identity(api_key: Option<String>) -> Result<TakoIdentity, String> {
    TakoService::current_identity(api_key).await
}

#[tauri::command]
pub async fn tako_logout() -> Result<bool, String> {
    TakoService::logout().await
}

#[tauri::command]
pub async fn tako_usage(api_key: String) -> Result<TakoUsage, String> {
    TakoService::usage(api_key).await
}

#[tauri::command]
pub async fn tako_list_models(api_key: String) -> Result<Vec<TakoModel>, String> {
    TakoService::list_models(api_key).await
}

async fn verify_tako_key(api_key: String) -> Result<TakoLoginResult, String> {
    let api_key = api_key.trim().to_string();
    if api_key.is_empty() {
        return Ok(TakoLoginResult {
            ok: false,
            error: Some("API Key 不能为空。".to_string()),
            ..Default::default()
        });
    }

    let client = reqwest::Client::new();
    let response = client
        .post(TAKO_IDENTITY_URL)
        .json(&json!({ "apiKey": api_key }))
        .send()
        .await
        .map_err(|err| format!("连接 Tako 失败：{err}"))?;

    let status = response.status();
    let body: Value = response
        .json()
        .await
        .map_err(|err| format!("解析 Tako 响应失败：{err}"))?;

    if !status.is_success() {
        return Ok(TakoLoginResult {
            ok: false,
            error: Some(format!("Tako 身份校验失败，HTTP 状态码：{status}。")),
            ..Default::default()
        });
    }

    if body.get("success").and_then(Value::as_bool) == Some(true) {
        let user = body.get("user");
        return Ok(TakoLoginResult {
            ok: true,
            name: user
                .and_then(|value| value.get("name"))
                .and_then(Value::as_str)
                .map(str::to_string),
            plan: user
                .and_then(|value| value.get("plan"))
                .and_then(|value| value.get("name").or(Some(value)))
                .or_else(|| user.and_then(|value| value.get("planName")))
                .or_else(|| body.get("plan"))
                .and_then(Value::as_str)
                .map(str::to_string),
            error: None,
        });
    }

    Ok(TakoLoginResult {
        ok: false,
        error: body
            .get("message")
            .or_else(|| body.get("error"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| Some("Tako API Key 无效。".to_string())),
        ..Default::default()
    })
}

async fn fetch_tako_usage(api_key: String) -> Result<TakoUsage, String> {
    let api_key = api_key.trim().to_string();
    if api_key.is_empty() {
        return Ok(TakoUsage {
            ok: false,
            error: Some("API Key 不能为空。".to_string()),
            ..Default::default()
        });
    }

    let client = reqwest::Client::new();
    let key_response: Value = client
        .post(format!("{TAKO_API_STATS_BASE}/get-key-id"))
        .json(&json!({ "apiKey": api_key }))
        .send()
        .await
        .map_err(|err| format!("连接 Tako 用量服务失败：{err}"))?
        .json()
        .await
        .map_err(|err| format!("解析 Tako Key 信息失败：{err}"))?;

    let api_id = key_response
        .get("data")
        .and_then(|value| value.get("id"))
        .and_then(Value::as_str);
    let Some(api_id) = api_id else {
        return Ok(TakoUsage {
            ok: false,
            error: Some("Tako API Key 无效，无法读取用量。".to_string()),
            ..Default::default()
        });
    };

    let quota_response: Value = client
        .get(format!("{TAKO_API_STATS_BASE}/user-quota?apiId={api_id}"))
        .send()
        .await
        .map_err(|err| format!("读取 Tako 用量失败：{err}"))?
        .json()
        .await
        .map_err(|err| format!("解析 Tako 用量响应失败：{err}"))?;

    let usage = quota_response.get("usage");
    let plan = quota_response.get("plan");

    Ok(TakoUsage {
        ok: true,
        window: TakoUsageWindow {
            used: number_field(usage, "windowCost"),
            limit: number_field(plan, "window_cost_limit"),
        },
        daily: TakoUsageWindow {
            used: number_field(usage, "dailyCost"),
            limit: number_field(plan, "daily_cost_limit"),
        },
        weekly: TakoUsageWindow {
            used: number_field(usage, "weeklyCost"),
            limit: number_field(plan, "weekly_cost_limit"),
        },
        plan_name: plan
            .and_then(|value| value.get("name"))
            .and_then(Value::as_str)
            .map(str::to_string),
        error: None,
    })
}

async fn list_tako_models(api_key: String) -> Result<Vec<TakoModel>, String> {
    let api_key = api_key.trim().to_string();
    if api_key.is_empty() {
        return Err("API Key 不能为空。".to_string());
    }

    let client = reqwest::Client::new();
    let response: Value = client
        .get(TAKO_MODELS_URL)
        .bearer_auth(api_key)
        .send()
        .await
        .map_err(|err| format!("读取 Tako 模型列表失败：{err}"))?
        .json()
        .await
        .map_err(|err| format!("解析 Tako 模型列表失败：{err}"))?;

    let data = response
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "Tako 模型响应中没有模型列表。".to_string())?;

    Ok(parse_models(data))
}

fn number_field(parent: Option<&Value>, key: &str) -> f64 {
    parent
        .and_then(|value| value.get(key))
        .and_then(|value| {
            value
                .as_f64()
                .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
        })
        .unwrap_or(0.0)
}

fn clients_for_provider(provider: &str) -> Vec<String> {
    let lowered = provider.to_lowercase();
    let mut clients = Vec::new();
    if lowered.contains("anthropic") || lowered.contains("claude") {
        clients.push("claude".to_string());
    }
    if lowered.contains("openai") || lowered.contains("gpt") || lowered.contains("codex") {
        clients.push("codex".to_string());
    }
    if lowered.contains("google") || lowered.contains("gemini") {
        clients.push("gemini".to_string());
    }
    clients
}

fn parse_models(data: &[Value]) -> Vec<TakoModel> {
    data.iter()
        .filter_map(|model| {
            let id = model.get("id").and_then(Value::as_str)?.to_string();
            let provider = model
                .get("owned_by")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            Some(TakoModel {
                name: id.clone(),
                clients: clients_for_provider(&provider),
                id,
                provider,
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "../tests/providers/tako.rs"]
mod tests;
