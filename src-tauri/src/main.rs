#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod alerts;
mod update;

use std::{collections::HashMap, path::PathBuf, sync::Mutex, time::Duration};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, RunEvent, State, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;
use tokio::task::AbortHandle;

use alerts::AlertPrefs;

const POLL: Duration = Duration::from_secs(3);
const ACTIVE: [&str; 4] = ["RUNNING", "PREPARE", "PAUSE", "SLICING"];
const DONE: [&str; 3] = ["FINISH", "FAILED", "IDLE"];

fn is_active(s: &str) -> bool {
    ACTIVE.contains(&s)
}

// Same file and keys as the Python version, so settings carry over.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
struct Config {
    url: String,
    api_key: String,
    printer_id: Option<i64>,
    auto_hide: bool,
    hide_delay_s: u64,
    alerts: AlertPrefs,
    check_updates: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config { url: String::new(), api_key: String::new(), printer_id: None, auto_hide: true, hide_delay_s: 60, alerts: AlertPrefs::default(), check_updates: true }
    }
}

struct AppState {
    cfg: Mutex<Config>,
    cfg_path: PathBuf,
    states: Mutex<HashMap<i64, String>>,
    shown: Mutex<Option<i64>>,
    last: Mutex<Option<Value>>,
    last_alert: Mutex<Option<String>>,
    hide: Mutex<Option<AbortHandle>>,
    alert_mem: Mutex<alerts::Memory>,
    update: Mutex<Option<update::Release>>,
    update_notified: Mutex<Option<String>>,
    http: reqwest::Client,
}

struct Api {
    base: String,
    key: String,
    http: reqwest::Client,
}

fn describe(e: reqwest::Error) -> String {
    if let Some(code) = e.status() {
        return match code.as_u16() {
            401 => "Bad API key".into(),
            403 => "API key lacks permission".into(),
            c => format!("HTTP {c}"),
        };
    }
    if e.is_connect() || e.is_timeout() {
        return "Can't reach Bambuddy".into();
    }
    e.to_string()
}

impl Api {
    fn new(url: &str, key: &str, http: &reqwest::Client) -> Api {
        Api { base: format!("{}/api/v1", url.trim().trim_end_matches('/')), key: key.trim().into(), http: http.clone() }
    }

    fn req(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let r = self.http.request(method, format!("{}{}", self.base, path));
        if self.key.is_empty() { r } else { r.bearer_auth(&self.key) }
    }

    async fn send(&self, method: reqwest::Method, path: &str) -> Result<Value, String> {
        let resp = self.req(method, path).send().await.map_err(describe)?.error_for_status().map_err(describe)?;
        Ok(resp.json().await.unwrap_or(Value::Null))
    }

    async fn get(&self, path: &str) -> Result<Value, String> {
        self.send(reqwest::Method::GET, path).await
    }

    async fn post(&self, path: &str) -> Result<Value, String> {
        self.send(reqwest::Method::POST, path).await
    }
}

fn api(state: &AppState) -> Api {
    let c = state.cfg.lock().unwrap();
    Api::new(&c.url, &c.api_key, &state.http)
}

// ---- commands ----

#[tauri::command]
fn get_config(state: State<AppState>) -> Config {
    state.cfg.lock().unwrap().clone()
}

#[tauri::command]
fn save_config(cfg: Config, state: State<AppState>) -> Result<(), String> {
    if let Some(dir) = state.cfg_path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    std::fs::write(&state.cfg_path, json).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&state.cfg_path, std::fs::Permissions::from_mode(0o600));
    }
    *state.cfg.lock().unwrap() = cfg;
    state.states.lock().unwrap().clear();
    *state.shown.lock().unwrap() = None;
    Ok(())
}

#[tauri::command]
async fn list_printers(url: String, key: String, state: State<'_, AppState>) -> Result<Value, String> {
    Api::new(&url, &key, &state.http).get("/printers/").await
}

#[tauri::command]
fn get_alert(state: State<AppState>) -> Option<String> {
    state.last_alert.lock().unwrap().clone()
}

#[tauri::command]
fn get_status(state: State<AppState>) -> Option<Value> {
    state.last.lock().unwrap().clone()
}

#[tauri::command]
async fn camera_url(state: State<'_, AppState>) -> Result<String, String> {
    let pid = (*state.shown.lock().unwrap()).ok_or("No printer yet")?;
    let a = api(&state);
    let token = a.post("/printers/camera/stream-token").await?;
    let token = token["token"].as_str().ok_or("No camera token")?;
    Ok(format!("{}/printers/{pid}/camera/stream?fps=8&token={token}", a.base))
}

#[tauri::command]
async fn stop_print(state: State<'_, AppState>) -> Result<(), String> {
    let pid = (*state.shown.lock().unwrap()).ok_or("No printer selected")?;
    api(&state).post(&format!("/printers/{pid}/print/stop")).await.map(|_| ())
}

// ---- windows ----

fn open_window(app: &AppHandle, label: &str, page: &str, title: &str, size: (f64, f64)) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, label, WebviewUrl::App(page.into()))
        .title(title)
        .inner_size(size.0, size.1)
        .build();
}

fn show_monitor(app: &AppHandle) {
    let st = app.state::<AppState>();
    {
        let mut shown = st.shown.lock().unwrap();
        if shown.is_none() {
            *shown = st.cfg.lock().unwrap().printer_id.or_else(|| st.states.lock().unwrap().keys().min().copied());
        }
    }
    open_window(app, "main", "index.html", "Bambuddy Monitor", (640.0, 560.0));
}

fn show_settings(app: &AppHandle) {
    open_window(app, "settings", "settings.html", "Bambuddy Monitor settings", (480.0, 660.0));
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        eprintln!("notification failed: {e}");
    }
}

#[tauri::command]
async fn test_alert(app: AppHandle) -> Result<(), String> {
    app.notification().builder().title("Bambuddy Monitor").body("Notifications work.").show().map_err(|e| e.to_string())
}

/// `manual`: the user asked (tray menu), so always answer and open the download page.
async fn check_updates(app: AppHandle, manual: bool) {
    let st = app.state::<AppState>();
    match update::latest(&st.http).await {
        Ok(Some(r)) if update::is_newer(update::CURRENT, &r.version) => {
            let first_time = st.update_notified.lock().unwrap().replace(r.version.clone()).as_deref() != Some(r.version.as_str());
            if manual || first_time {
                notify(&app, "Update available", &format!("Bambuddy Monitor {} is out (you have {}).", r.version, update::CURRENT));
            }
            if manual {
                let _ = app.opener().open_url(&r.url, None::<&str>);
            }
            *st.update.lock().unwrap() = Some(r);
        }
        Ok(_) if manual => notify(&app, "Bambuddy Monitor", &format!("You're up to date (v{}).", update::CURRENT)),
        Err(e) if manual => notify(&app, "Couldn't check for updates", &e),
        _ => {}
    }
}

async fn update_loop(app: AppHandle) {
    tokio::time::sleep(Duration::from_secs(30)).await;
    loop {
        if app.state::<AppState>().cfg.lock().unwrap().check_updates {
            check_updates(app.clone(), false).await;
        }
        tokio::time::sleep(Duration::from_secs(24 * 3600)).await;
    }
}

// ---- polling ----

async fn fetch_statuses(a: &Api, only: Option<i64>) -> Result<Vec<(i64, Value)>, String> {
    let ids: Vec<i64> = match only {
        Some(id) => vec![id],
        None => a.get("/printers/").await?.as_array().map(|v| v.iter().filter_map(|p| p["id"].as_i64()).collect()).unwrap_or_default(),
    };
    let mut out = Vec::new();
    for id in ids {
        let mut s = a.get(&format!("/printers/{id}/status")).await?;
        if s["name"].is_null() {
            s["name"] = format!("Printer {id}").into();
        }
        out.push((id, s));
    }
    Ok(out)
}

fn handle_statuses(app: &AppHandle, cfg: &Config, statuses: Vec<(i64, Value)>) {
    let st = app.state::<AppState>();
    for (pid, s) in statuses {
        let new = s["state"].as_str().unwrap_or("").to_string();
        let old = st.states.lock().unwrap().insert(pid, new.clone());
        let found = alerts::detect(&cfg.alerts, pid, old.as_deref(), &s, &mut st.alert_mem.lock().unwrap());
        for a in found {
            notify(app, &a.title, &a.body);
            let line = format!("{}: {}", a.title, a.body);
            *st.last_alert.lock().unwrap() = Some(line.clone());
            let _ = app.emit_to("main", "alert", line);
            if a.needs_you && cfg.alerts.open_window && app.get_webview_window("main").is_none() {
                *st.shown.lock().unwrap() = Some(pid);
                *st.last.lock().unwrap() = Some(s.clone());
                show_monitor(app);
            }
        }
        if !s["connected"].as_bool().unwrap_or(false) {
            continue;
        }
        let window_open = app.get_webview_window("main").is_some();
        let started = is_active(&new) && old.as_deref().is_some_and(|o| !is_active(o));
        let first_active = old.is_none() && is_active(&new) && !window_open;
        let tracked = {
            let mut shown = st.shown.lock().unwrap();
            if started || first_active {
                *shown = Some(pid);
            } else if shown.is_none() && is_active(&new) {
                *shown = Some(pid);
            }
            *shown == Some(pid)
        };
        if started || first_active {
            if let Some(h) = st.hide.lock().unwrap().take() {
                h.abort();
            }
            *st.last.lock().unwrap() = Some(s.clone());
            show_monitor(app);
            let _ = app.emit_to("main", "status", &s);
        } else if tracked {
            *st.last.lock().unwrap() = Some(s.clone());
            let _ = app.emit_to("main", "status", &s);
            let ended = old.as_deref().is_some_and(is_active) && DONE.contains(&new.as_str());
            if ended && window_open {
                if cfg.auto_hide {
                    let app2 = app.clone();
                    let delay = Duration::from_secs(cfg.hide_delay_s);
                    let h = tauri::async_runtime::spawn(async move {
                        tokio::time::sleep(delay).await;
                        if let Some(w) = app2.get_webview_window("main") {
                            let _ = w.close();
                        }
                    });
                    if let Some(old) = st.hide.lock().unwrap().replace(h.inner().abort_handle()) {
                        old.abort();
                    }
                }
            }
        }
    }
}

async fn poll_loop(app: AppHandle) {
    let mut err_notified = false;
    loop {
        let (cfg, a) = {
            let st = app.state::<AppState>();
            let cfg = st.cfg.lock().unwrap().clone();
            let a = Api::new(&cfg.url, &cfg.api_key, &st.http);
            (cfg, a)
        };
        if !cfg.url.is_empty() {
            match fetch_statuses(&a, cfg.printer_id).await {
                Ok(statuses) => {
                    err_notified = false;
                    handle_statuses(&app, &cfg, statuses);
                }
                Err(e) => {
                    if !err_notified {
                        notify(&app, "Bambuddy Monitor", &e);
                    }
                    err_notified = true;
                }
            }
        }
        tokio::time::sleep(POLL).await;
    }
}

fn load_config(path: &PathBuf) -> Config {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_monitor(app)))
        .on_window_event(|window, event| {
            // Bambu printers allow one camera connection: release it when the monitor window goes away.
            if window.label() == "main" && matches!(event, WindowEvent::Destroyed) {
                let app = window.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    let st = app.state::<AppState>();
                    let pid = *st.shown.lock().unwrap();
                    if let Some(pid) = pid {
                        let _ = api(&st).post(&format!("/printers/{pid}/camera/stop")).await;
                    }
                });
            }
        })
        .invoke_handler(tauri::generate_handler![get_alert, test_alert, get_config, save_config, list_printers, get_status, camera_url, stop_print])
        .setup(|app| {
            let cfg_path = app.path().config_dir()?.join("bambuddy-monitor").join("config.json");
            let cfg = load_config(&cfg_path);
            let first_run = cfg.url.is_empty();
            let http = reqwest::Client::builder().timeout(Duration::from_secs(10)).build()?;
            app.manage(AppState {
                cfg: Mutex::new(cfg),
                cfg_path,
                states: Mutex::new(HashMap::new()),
                shown: Mutex::new(None),
                last: Mutex::new(None),
                last_alert: Mutex::new(None),
                hide: Mutex::new(None),
                alert_mem: Mutex::new(alerts::Memory::default()),
                update: Mutex::new(None),
                update_notified: Mutex::new(None),
                http,
            });

            let version = MenuItem::with_id(app, "version", format!("Bambuddy Monitor v{}", update::CURRENT), false, None::<&str>)?;
            let updates = MenuItem::with_id(app, "updates", "Check for updates…", true, None::<&str>)?;
            let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&version, &PredefinedMenuItem::separator(app)?, &show, &settings, &updates, &PredefinedMenuItem::separator(app)?, &quit])?;
            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Bambuddy Monitor")
                .menu(&menu)
                .on_menu_event(|app, ev| match ev.id().as_ref() {
                    "show" => show_monitor(app),
                    "settings" => show_settings(app),
                    "updates" => {
                        tauri::async_runtime::spawn(check_updates(app.clone(), true));
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            if first_run {
                show_settings(app.handle());
            }
            tauri::async_runtime::spawn(poll_loop(app.handle().clone()));
            tauri::async_runtime::spawn(update_loop(app.handle().clone()));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build Bambuddy Monitor");

    app.run(|_app, event| {
        // A tray app: closing the last window must not quit.
        if let RunEvent::ExitRequested { api, code, .. } = event {
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}
