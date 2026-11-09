//! Microsoft sign-in for Minecraft Java Edition (official device-code flow).
//!
//! Chain: Microsoft OAuth -> Xbox Live -> XSTS -> Minecraft services -> Minecraft profile.
//!
//! What is stored, and where:
//! - The Microsoft refresh token is stored in Windows Credential Manager. It is the only
//!   long-lived secret. Passwords are never seen by Aurora.
//! - The Minecraft access token (valid about 24 hours) is kept in memory only. It is
//!   refreshed silently from the refresh token before it expires.
//!
//! Owning the game is checked with GET /minecraft/profile. Accounts without a Java
//! profile get HTTP 404, and Aurora refuses to start the game for them.
//!
//! Sign-in needs an Azure client ID that Mojang has approved for the Minecraft services
//! API. Until MS_CLIENT_ID is replaced, sign-in stops with a clear message.

use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::minecraft::Session;

/// Replace with the client ID of your Azure app once Mojang has approved it. See AUTH.md.
const MS_CLIENT_ID: &str = "dd7b50cd-92f1-476e-9ec8-13c35423a0db";
const MS_SCOPE: &str = "XboxLive.signin offline_access";
const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBL_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTH_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";
const KEYRING_SERVICE: &str = "AuroraLauncher";
const KEYRING_USER: &str = "microsoft-refresh-token";
/// Refresh the Minecraft token when less than 5 minutes remain.
const REFRESH_MARGIN_SECS: u64 = 300;

#[derive(Clone)]
struct Account {
    name: String,
    uuid: String,
    access_token: String,
    expires_at: u64,
}

static ACCOUNT: Mutex<Option<Account>> = Mutex::new(None);

/// What the UI shows about the signed-in account.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountInfo {
    pub signed_in: bool,
    pub name: String,
    pub uuid: String,
    pub expires_at: u64,
}

/// The code the player types on the Microsoft page.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCodePrompt {
    pub user_code: String,
    pub verification_uri: String,
}

#[derive(Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    expires_in: u64,
    interval: u64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct XboxToken {
    token: String,
    display_claims: XuiClaims,
}

#[derive(Deserialize)]
struct XuiClaims {
    xui: Vec<Uhs>,
}

#[derive(Deserialize)]
struct Uhs {
    uhs: String,
}

#[derive(Deserialize)]
struct McLogin {
    access_token: String,
    expires_in: u64,
}

#[derive(Deserialize)]
struct McProfile {
    id: String,
    name: String,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("Aurora/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

fn net_err(e: reqwest::Error) -> String {
    format!("Could not reach Microsoft or Minecraft services. Check your internet connection. ({e})")
}

fn ensure_client_id() -> Result<(), String> {
    if MS_CLIENT_ID.starts_with("REPLACE_WITH") {
        Err("Microsoft sign-in is not set up yet. The Azure client ID has not been added. See AUTH.md.".to_string())
    } else {
        Ok(())
    }
}

fn set_account(account: Option<Account>) {
    if let Ok(mut guard) = ACCOUNT.lock() {
        *guard = account;
    }
}

fn current_account() -> Option<Account> {
    let guard = ACCOUNT.lock().ok()?;
    let account: Option<Account> = (*guard).clone();
    account
}

fn store_refresh(token: &str) -> Result<(), String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|e| format!("Could not open Windows Credential Manager: {e}"))?;
    let result = entry
        .set_password(token)
        .map_err(|e| format!("Could not save the sign-in to Windows Credential Manager: {e}"));
    result
}

fn load_refresh() -> Option<String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).ok()?;
    let token = entry.get_password().ok();
    token
}

fn delete_refresh() {
    if let Ok(entry) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER) {
        let _ = entry.delete_credential();
    }
}

/// Xbox Live -> XSTS -> Minecraft services -> profile. Fails if the account does not own the game.
async fn minecraft_login(client: &reqwest::Client, ms_access: &str) -> Result<Account, String> {
    // 1. Xbox Live
    let xbl_body = json!({
        "Properties": {
            "AuthMethod": "RPS",
            "SiteName": "user.auth.xboxlive.com",
            "RpsTicket": format!("d={ms_access}")
        },
        "RelyingParty": "http://auth.xboxlive.com",
        "TokenType": "JWT"
    });
    let xbl_resp = client
        .post(XBL_AUTH_URL)
        .header("Accept", "application/json")
        .json(&xbl_body)
        .send()
        .await
        .map_err(net_err)?;
    if !xbl_resp.status().is_success() {
        return Err("Xbox Live rejected the sign-in. Try again in a few minutes.".to_string());
    }
    let xbl: XboxToken = xbl_resp
        .json()
        .await
        .map_err(|e| format!("Unexpected reply from Xbox Live: {e}"))?;
    let uhs = xbl
        .display_claims
        .xui
        .first()
        .map(|x| x.uhs.clone())
        .ok_or_else(|| "Xbox Live reply did not include a user hash.".to_string())?;

    // 2. XSTS
    let xsts_body = json!({
        "Properties": {
            "SandboxId": "RETAIL",
            "UserTokens": [xbl.token.clone()]
        },
        "RelyingParty": "rp://api.minecraftservices.com/",
        "TokenType": "JWT"
    });
    let xsts_resp = client
        .post(XSTS_AUTH_URL)
        .json(&xsts_body)
        .send()
        .await
        .map_err(net_err)?;
    if !xsts_resp.status().is_success() {
        let body: Value = xsts_resp.json().await.unwrap_or(Value::Null);
        let code = body.get("XErr").and_then(Value::as_u64).unwrap_or(0);
        let message = match code {
            2148916233 => "This Microsoft account has no Xbox profile. Create one at xbox.com, then sign in again.",
            2148916238 => "This is a child account. An adult must add it to a family group before it can sign in.",
            _ => "Xbox Live refused the sign-in for this account.",
        };
        return Err(message.to_string());
    }
    let xsts: XboxToken = xsts_resp
        .json()
        .await
        .map_err(|e| format!("Unexpected reply from Xbox Live (XSTS): {e}"))?;

    // 3. Minecraft services
    let identity = format!("XBL3.0 x={uhs};{}", xsts.token);
    let mc_resp = client
        .post(MC_LOGIN_URL)
        .json(&json!({ "identityToken": identity }))
        .send()
        .await
        .map_err(net_err)?;
    if !mc_resp.status().is_success() {
        return Err(format!(
            "Minecraft services refused the login (HTTP {}). The Azure client ID must be approved for the Minecraft API. See AUTH.md.",
            mc_resp.status()
        ));
    }
    let mc: McLogin = mc_resp
        .json()
        .await
        .map_err(|e| format!("Unexpected reply from Minecraft services: {e}"))?;

    // 4. Profile. A 404 means the account has no Java profile, so it does not own the game.
    let profile_resp = client
        .get(MC_PROFILE_URL)
        .bearer_auth(&mc.access_token)
        .send()
        .await
        .map_err(net_err)?;
    if profile_resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err("This Microsoft account does not own Minecraft Java Edition. Aurora only starts the game for owners.".to_string());
    }
    if !profile_resp.status().is_success() {
        return Err(format!(
            "Could not read the Minecraft profile (HTTP {}).",
            profile_resp.status()
        ));
    }
    let profile: McProfile = profile_resp
        .json()
        .await
        .map_err(|e| format!("Unexpected reply from the Minecraft profile service: {e}"))?;

    Ok(Account {
        name: profile.name,
        uuid: profile.id,
        access_token: mc.access_token,
        expires_at: now_secs() + mc.expires_in,
    })
}

/// Polls Microsoft until the player approves the code, then signs in and stores the refresh token.
async fn poll_device_code(app: &AppHandle, client: &reqwest::Client, dc: DeviceCodeResponse) -> Result<(), String> {
    let deadline = now_secs() + dc.expires_in;
    let mut interval = dc.interval.max(3);
    loop {
        if now_secs() >= deadline {
            return Err("The sign-in code expired. Start sign-in again.".to_string());
        }
        tokio::time::sleep(Duration::from_secs(interval)).await;

        let resp = client
            .post(TOKEN_URL)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("client_id", MS_CLIENT_ID),
                ("device_code", dc.device_code.as_str()),
            ])
            .send()
            .await
            .map_err(net_err)?;
        let body: TokenResponse = resp
            .json()
            .await
            .map_err(|e| format!("Unexpected reply from Microsoft: {e}"))?;

        if let Some(err) = body.error.as_deref() {
            match err {
                "authorization_pending" => continue,
                "slow_down" => {
                    interval += 5;
                    continue;
                }
                "authorization_declined" => return Err("Sign-in was cancelled.".to_string()),
                "expired_token" => return Err("The sign-in code expired. Start sign-in again.".to_string()),
                other => return Err(format!("Microsoft sign-in failed: {other}")),
            }
        }

        let ms_access = body
            .access_token
            .ok_or_else(|| "Microsoft did not return a token.".to_string())?;
        let refresh = body
            .refresh_token
            .ok_or_else(|| "Microsoft did not return a refresh token.".to_string())?;
        let account = minecraft_login(client, &ms_access).await?;
        store_refresh(&refresh)?;
        set_account(Some(account));
        let _ = app.emit("auth-changed", ());
        return Ok(());
    }
}

/// Starts sign-in. Returns the code to show the player. Sign-in finishes in the background
/// and reports "auth-changed" or "auth-failed" to the UI.
pub async fn start_sign_in(app: AppHandle) -> Result<DeviceCodePrompt, String> {
    ensure_client_id()?;
    let client = http();
    let resp = client
        .post(DEVICE_CODE_URL)
        .form(&[("client_id", MS_CLIENT_ID), ("scope", MS_SCOPE)])
        .send()
        .await
        .map_err(net_err)?;
    if !resp.status().is_success() {
        return Err(format!("Microsoft refused the sign-in request (HTTP {}).", resp.status()));
    }
    let dc: DeviceCodeResponse = resp
        .json()
        .await
        .map_err(|e| format!("Unexpected reply from Microsoft: {e}"))?;

    let prompt = DeviceCodePrompt {
        user_code: dc.user_code.clone(),
        verification_uri: dc.verification_uri.clone(),
    };

    tauri::async_runtime::spawn(async move {
        match poll_device_code(&app, &client, dc).await {
            Ok(()) => {}
            Err(message) => {
                let _ = app.emit("auth-failed", message);
            }
        }
    });

    Ok(prompt)
}

/// Gets a fresh Minecraft token, refreshing it when needed.
async fn ensure_account() -> Result<Account, String> {
    if let Some(account) = current_account() {
        if account.expires_at > now_secs() + REFRESH_MARGIN_SECS {
            return Ok(account);
        }
    }
    ensure_client_id()?;
    let refresh = load_refresh().ok_or_else(|| "Sign in with Microsoft first.".to_string())?;
    let client = http();
    let resp = client
        .post(TOKEN_URL)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", MS_CLIENT_ID),
            ("refresh_token", refresh.as_str()),
            ("scope", MS_SCOPE),
        ])
        .send()
        .await
        .map_err(net_err)?;
    let body: TokenResponse = resp
        .json()
        .await
        .map_err(|e| format!("Unexpected reply from Microsoft: {e}"))?;

    if let Some(code) = body.error.clone() {
        if code == "invalid_grant" {
            delete_refresh();
            set_account(None);
            return Err("Your Microsoft sign-in expired. Sign in again.".to_string());
        }
        return Err(format!("Could not refresh the sign-in: {code}"));
    }

    let ms_access = body
        .access_token
        .ok_or_else(|| "Microsoft did not return a token.".to_string())?;
    if let Some(new_refresh) = body.refresh_token.as_deref() {
        store_refresh(new_refresh)?;
    }
    let account = minecraft_login(&client, &ms_access).await?;
    set_account(Some(account.clone()));
    Ok(account)
}

/// Current account state for the UI. Signed out when there is no stored sign-in.
pub async fn status() -> Result<AccountInfo, String> {
    let signed_out = AccountInfo {
        signed_in: false,
        name: String::new(),
        uuid: String::new(),
        expires_at: 0,
    };
    if current_account().is_none() && load_refresh().is_none() {
        return Ok(signed_out);
    }
    match ensure_account().await {
        Ok(a) => Ok(AccountInfo {
            signed_in: true,
            name: a.name,
            uuid: a.uuid,
            expires_at: a.expires_at,
        }),
        Err(message) => {
            if load_refresh().is_none() {
                Ok(signed_out)
            } else {
                Err(message)
            }
        }
    }
}

/// Session used when starting Minecraft.
pub async fn launch_session() -> Result<Session, String> {
    let account = ensure_account().await?;
    Ok(Session {
        username: account.name,
        uuid: account.uuid,
        access_token: account.access_token,
        user_type: "msa".to_string(),
    })
}

/// Removes the stored sign-in and the in-memory token.
pub fn sign_out() {
    delete_refresh();
    set_account(None);
}
