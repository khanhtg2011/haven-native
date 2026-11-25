use crate::net::models::{LoginResponse, PublicConfig, UploadResponse};
use reqwest::multipart::{Form, Part};
use reqwest::Client;
use serde_json::json;
use std::time::Duration;

/// Strict TLS certificate verification is enabled by default. Opt into invalid
/// self-signed local dev certificates only when `HAVEN_INSECURE_TLS` is set.
pub fn should_accept_invalid_certs(env_val: Option<&str>) -> bool {
    matches!(
        env_val.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1" | "true" | "yes")
    )
}

pub struct HavenApiClient {
    client: Client,
    base_url: String,
}

impl HavenApiClient {
    pub fn new(base_url: impl Into<String>) -> Self {
        let mut builder = Client::builder()
            .pool_idle_timeout(Duration::from_secs(60))
            .connect_timeout(Duration::from_secs(10));
        if should_accept_invalid_certs(std::env::var("HAVEN_INSECURE_TLS").ok().as_deref()) {
            log::warn!("HAVEN_INSECURE_TLS enabled: skipping TLS certificate validation");
            builder = builder.danger_accept_invalid_certs(true);
        }
        let client = builder.build().unwrap_or_default();

        Self {
            client,
            base_url: base_url.into(),
        }
    }

    pub fn client(&self) -> Client {
        self.client.clone()
    }

    pub async fn get_public_config(&self) -> Result<PublicConfig, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/api/public-config", self.base_url);
        let resp = self.client.get(&url).timeout(Duration::from_secs(15)).send().await?;
        let config: PublicConfig = resp.json().await?;
        Ok(config)
    }

    pub async fn login(
        &self,
        username: &str,
        password: &str,
    ) -> Result<LoginResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/api/auth/login", self.base_url);
        let payload = json!({
            "username": username,
            "password": password,
            "eulaVersion": "2.0",
            "ageVerified": true
        });

        let resp = self.client.post(&url).timeout(Duration::from_secs(15)).json(&payload).send().await?;
        if !resp.status().is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(format!("Login failed: {}", err_text).into());
        }

        let login_res: LoginResponse = resp.json().await?;
        Ok(login_res)
    }

    pub async fn upload_file(
        &self,
        token: &str,
        file_name: &str,
        file_bytes: Vec<u8>,
    ) -> Result<UploadResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/api/upload-file", self.base_url);
        let part = Part::bytes(file_bytes)
            .file_name(file_name.to_string());
        let form = Form::new().part("file", part);

        let resp = self.client
            .post(&url)
            .timeout(Duration::from_secs(120))
            .header("Authorization", format!("Bearer {}", token))
            .multipart(form)
            .send()
            .await?;

        if !resp.status().is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(format!("Upload failed: {}", err_text).into());
        }

        let upload_res: UploadResponse = resp.json().await?;
        Ok(upload_res)
    }

    #[allow(dead_code)]
    pub async fn register(
        &self,
        username: &str,
        password: &str,
    ) -> Result<LoginResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/api/auth/register", self.base_url);
        let payload = json!({
            "username": username,
            "password": password,
            "eulaVersion": "2.0",
            "ageVerified": true
        });

        let resp = self.client.post(&url).json(&payload).send().await?;
        if !resp.status().is_success() {
            let err_text = resp.text().await.unwrap_or_default();
            return Err(format!("Registration failed: {}", err_text).into());
        }

        let login_res: LoginResponse = resp.json().await?;
        Ok(login_res)
    }
}
