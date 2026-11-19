//! `wireguard_sync/__init__.py` APIClient + `rest_api.py` types — aiohttp →
//! reqwest, `X-API-TOKEN` header, concurrency-limited by a semaphore, errors
//! wrapped as `ApiError: ...` (4xx pass through with their status).

use crate::settings::Settings;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// `rest_api.py` types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeySlot {
    pub public_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiPeer {
    pub key_slot: KeySlot,
    pub cidr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinimalInterface {
    pub id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interface {
    pub id: i64,
    pub cidr: String,
    #[serde(default)]
    pub public_key: Option<String>,
    #[serde(default)]
    pub last_modified: Option<String>,
    #[serde(default)]
    pub peers: Vec<ApiPeer>,
    pub port: i64,
}

pub struct ApiClient {
    base_url: String,
    http: reqwest::Client,
    semaphore: Arc<Semaphore>,
}

/// `reraise_api_errors` — connection errors and 5xx become `ApiError: ...`;
/// 4xx keep their status (we should not receive any 4xx errors).
fn wrap_status_err(e: reqwest::Error) -> String {
    if let Some(status) = e.status() {
        if status.as_u16() < 500 {
            return format!("ClientResponseError: {status}");
        }
    }
    format!("{}{e}", crate::ERR_API)
}

impl ApiClient {
    pub fn new(settings: Settings, semaphore: Arc<Semaphore>) -> ApiClient {
        let origin = crate::force_schema(&settings.api_server);
        let base = if settings.api_base.ends_with('/') {
            settings.api_base.clone()
        } else {
            format!("{}/", settings.api_base)
        };
        let mut headers = HeaderMap::new();
        if let Some(token) = &settings.api_token {
            if let Ok(v) = reqwest::header::HeaderValue::from_str(token) {
                headers.insert("X-API-TOKEN", v);
            }
        }
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .build()
            .expect("reqwest client builds");
        ApiClient {
            base_url: format!("{origin}{base}"),
            http,
            semaphore,
        }
    }

    fn url(&self, stub: &str) -> String {
        format!("{}{}", self.base_url, stub)
    }

    async fn permit(&self) -> OwnedSemaphorePermit {
        self.semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("semaphore not closed")
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, url: String) -> Result<T, String> {
        let _p = self.permit().await;
        let resp = self.http.get(&url).send().await.map_err(wrap_status_err)?;
        let resp = resp.error_for_status().map_err(wrap_status_err)?;
        resp.json().await.map_err(wrap_status_err)
    }

    /// `get_interfaces_to_update` — `?need_sync=` unless `all`.
    pub async fn get_interfaces_to_update(
        &self,
        all: bool,
    ) -> Result<Vec<MinimalInterface>, String> {
        let mut url = self.url("interfaces");
        if !all {
            url.push_str("?need_sync=");
        }
        self.get_json(url).await
    }

    pub async fn get_interface(&self, interface_id: i64) -> Result<Interface, String> {
        self.get_json(self.url(&format!("interface/{interface_id}")))
            .await
    }

    /// `ack_interface` — form POST `version=<last_modified>`.
    pub async fn ack_interface(
        &self,
        interface_id: i64,
        last_modified: Option<String>,
    ) -> Result<(), String> {
        let _p = self.permit().await;
        let url = self.url(&format!("interface/{interface_id}/sync"));
        let mut form: Vec<(&str, String)> = vec![];
        if let Some(v) = last_modified {
            form.push(("version", v));
        }
        let resp = self
            .http
            .post(&url)
            .form(&form)
            .send()
            .await
            .map_err(wrap_status_err)?;
        resp.error_for_status().map_err(wrap_status_err)?;
        Ok(())
    }

    /// `set_pubk` — form PUT `public_key=<key>`.
    pub async fn set_pubk(&self, interface_id: i64, public_key: &str) -> Result<(), String> {
        let _p = self.permit().await;
        let url = self.url(&format!("interface/{interface_id}"));
        let resp = self
            .http
            .put(&url)
            .form(&[("public_key", public_key)])
            .send()
            .await
            .map_err(wrap_status_err)?;
        resp.error_for_status().map_err(wrap_status_err)?;
        Ok(())
    }
}
