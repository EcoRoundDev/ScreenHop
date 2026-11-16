use anyhow::Result;
use muda::{Menu, MenuItem, PredefinedMenuItem};
use screenhop::config::AppConfig;
use screenhop_platform::{AutoStart, AutoStartStatus, BlockReason};
use slint::ComponentHandle;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use tray_icon::{menu::MenuEvent, Icon, TrayIconBuilder};

const MENU_ID_TOGGLE: &str = "toggle";
const MENU_ID_SETTINGS: &str = "settings";
const MENU_ID_AUTOSTART: &str = "autostart";
const MENU_ID_CHECK_UPDATE: &str = "check_update";
const MENU_ID_PROXY_ENABLE: &str = "proxy_enable";
const MENU_ID_PROXY_SETTINGS: &str = "proxy_settings";
const MENU_ID_AUTO_CHECK_UPDATE: &str = "auto_check_update";
const MENU_ID_QUIT: &str = "quit";
#[cfg(target_os = "macos")]
const MENU_ID_PERMISSIONS: &str = "permissions";

const REPO_URL: &str = "https://github.com/EcoRoundDev/ScreenHop";

/// 创建托盘图标（使用真实的 png）
fn create_tray_icon_image() -> Icon {
    #[cfg(target_os = "macos")]
    let icon_data = include_bytes!("../assets/icon-mac-tray.png");
    #[cfg(not(target_os = "macos"))]
    let icon_data = include_bytes!("../assets/icon-32.png");

    let image = image::load_from_memory(icon_data)
        .expect("无法加载托盘图标数据")
        .into_rgba8();
    let (width, height) = image.dimensions();
    let rgba = image.into_raw();
    Icon::from_rgba(rgba, width, height).expect("创建托盘图标失败")
}

use std::sync::atomic::{AtomicBool, Ordering};

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn should_auto_check(cfg: &AppConfig) -> bool {
    if screenhop::is_dev_build() {
        return false;
    }
    if cfg.test_update_version.is_some() {
        return true;
    }
    if !cfg.auto_check_update {
        return false;
    }
    match cfg.last_update_check_ts {
        Some(ts) => now_unix().saturating_sub(ts) >= 24 * 60 * 60,
        None => true,
    }
}

fn toast(window: &crate::slint_ui::MainWindow, text: &str) {
    window.set_toast_text(text.into());
    window.set_toast_visible(true);
}

fn current_autostart_status() -> Result<AutoStartStatus> {
    if screenhop::is_dev_build() {
        Ok(AutoStartStatus::disabled("开发版不启用开机自启动"))
    } else {
        screenhop_platform::auto_start().status()
    }
}

#[cfg(target_os = "macos")]
fn permission_text(status: &crate::macos_runtime::HookStatus) -> (i32, &'static str, String) {
    use crate::macos_runtime::HookState;
    match status.state {
        HookState::NeedsPermission => (0, "需要辅助功能权限", format!("需要辅助功能权限，才能监听中键并移动窗口。请在系统设置中开启 {}，授权后将自动生效。", screenhop::app_name())),
        HookState::Paused => (1, "已暂停", "辅助功能权限已就绪，打开上方开关即可启用。".into()),
        HookState::Starting => (2, "正在启动", "正在安装鼠标中键监听。".into()),
        HookState::Running => (3, "中键移动运行中", "辅助功能权限已就绪。关闭开关可立即暂停。".into()),
        HookState::Failed => (4, "启动失败", format!("{}。将有限重试；仍未恢复时，可重试或重新启动。", status.error.as_deref().unwrap_or("鼠标监听未能启动"))),
    }
}

#[cfg(target_os = "macos")]
fn apply_permission_status(
    window: &crate::slint_ui::MainWindow,
    status: &crate::macos_runtime::HookStatus,
) {
    let (state, text, detail) = permission_text(status);
    window.set_permission_state(state);
    window.set_permission_status(text.into());
    window.set_permission_detail(detail.into());
}

#[cfg(target_os = "macos")]
fn reconcile_permissions(
    config: &Arc<Mutex<AppConfig>>,
    items: &MenuItems,
    force: bool,
) -> Option<crate::macos_runtime::HookStatus> {
    let cfg = config.lock().ok()?.clone();
    let status = crate::macos_runtime::reconcile(&cfg, force);
    items.status_item.set_text(format!(
        "{}：{}",
        screenhop::app_name(),
        permission_text(&status).1
    ));
    Some(status)
}

fn autostart_detail(status: &AutoStartStatus) -> String {
    match &status.blocked {
        Some(BlockReason::DisabledBySystem { since }) => match since {
            Some(since) => format!("已被系统禁用（{since} 起），点「修复」重新启用"),
            None => "已被系统禁用，点「修复」重新启用".to_string(),
        },
        Some(BlockReason::TargetMissing { path }) => format!("注册的路径已失效：{path}"),
        Some(BlockReason::NeedsApproval) => status.detail.clone(),
        Some(BlockReason::PermissionDenied) => status.detail.clone(),
        Some(BlockReason::Unsupported) => status.detail.clone(),
        None => status.detail.clone(),
    }
}

fn autostart_blocked(status: &AutoStartStatus) -> bool {
    status.blocked.is_some() || status.requires_user_action
}

fn parse_proxy_url(url: &str) -> (String, String, String) {
    let mut protocol = "SOCKS5".to_string();
    let mut address = "127.0.0.1".to_string();
    let mut port = "2888".to_string();
    if url.is_empty() {
        return (protocol, address, port);
    }
    let (scheme, rest): (&str, &str) = url.split_once("://").unwrap_or(("socks5", url));
    protocol = match scheme.to_ascii_lowercase().as_str() {
        "socks4" | "socks4a" => "SOCKS4",
        "https" => "HTTPS",
        "http" => "HTTP",
        _ => "SOCKS5",
    }
    .to_string();
    if let Some((addr, p)) = rest.split_once(':') {
        address = addr.to_string();
        port = p.to_string();
    } else {
        address = rest.to_string();
    }
    (protocol, address, port)
}

fn protocol_to_scheme(proto: &str) -> &'static str {
    match proto {
        "SOCKS4" => "socks4",
        "HTTPS" => "https",
        "HTTP" => "http",
        _ => "socks5",
    }
}

/// 在后台线程中执行更新检查和下载
fn do_check_update(config: Arc<Mutex<AppConfig>>, manual: bool, startup: bool) {
    if screenhop::is_dev_build() {
        return;
    }
    let (
        version,
        proxy_url,
        proxy_username,
        proxy_password,
        mock_version,
        mock_url,
        skip_version,
        dark,
    ) = {
        let cfg = match config.lock() {
            Ok(cfg) => cfg,
            Err(_) => return,
        };
        let (proxy_url, proxy_username, proxy_password) =
            if cfg.proxy_enabled && !cfg.proxy_url.is_empty() {
                (
                    Some(cfg.proxy_url.clone()),
                    cfg.proxy_username.clone(),
                    cfg.proxy_password.clone(),
                )
            } else {
                (None, None, None)
            };
        (
            env!("CARGO_PKG_VERSION").to_string(),
            proxy_url,
            proxy_username,
            proxy_password,
            cfg.test_update_version.clone(),
            cfg.test_update_url.clone(),
            cfg.skip_update_version.clone(),
            crate::slint_ui::effective_dark(cfg.theme),
        )
    };
    std::thread::spawn(move || {
        if startup {
            std::thread::sleep(std::time::Duration::from_secs(3));
        }
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            match screenhop::updater::check_for_update_with_mock(
                &version,
                proxy_url.as_deref(),
                proxy_username.as_deref(),
                proxy_password.as_deref(),
                mock_version.as_deref(),
                mock_url.as_deref(),
            )
            .await
            {
                Ok(result) => {
                    if let Some(err) = &result.error {
                        log::warn!("检查更新失败: {}", err);
                        if manual {
                            crate::slint_ui::show_error("检查更新失败", &err.to_string());
                        }
                        return;
                    }

                    if let Ok(mut cfg) = config.lock() {
                        cfg.last_update_check_ts = Some(now_unix());
                        if let Err(err) = cfg.save() {
                            log::error!("保存配置失败: {}", err);
                        }
                    }

                    if skip_version.as_deref() == Some(result.latest_version.as_str()) {
                        log::info!("跳过版本 {}: 用户已选择跳过", result.latest_version);
                        return;
                    }

                    if result.has_update {
                        log::info!(
                            "发现新版本: {} → {}",
                            result.current_version,
                            result.latest_version
                        );

                        let prompt =
                            format!("发现新版本 {}，是否立即下载并安装？", result.latest_version);
                        let should_update =
                            crate::slint_ui::confirm("发现更新", &prompt, true, "立即更新")
                                .recv_timeout(std::time::Duration::from_secs(600))
                                .unwrap_or(false);

                        if !should_update {
                            log::info!("用户取消了更新");
                            return;
                        }

                        // 2. 显示进度对话框并开始下载
                        if let Some(download_url) = result.download_url {
                            let proxy_url_clone2 = proxy_url.clone();
                            let proxy_username_clone2 = proxy_username.clone();
                            let proxy_password_clone2 = proxy_password.clone();
                            let _ = slint::invoke_from_event_loop(move || {
                                // 在主线程创建并显示进度框
                                if let Ok(dialog) = crate::slint_ui::UpdateProgressDialog::new() {
                                    crate::slint_ui::apply_theme_progress(&dialog, dark);

                                    dialog.set_status_text("准备下载...".into());
                                    dialog.set_progress(0.0);
                                    dialog.set_can_cancel(true);

                                    let dialog_weak = dialog.as_weak();

                                    // 标记是否取消下载
                                    let is_cancelled = Arc::new(AtomicBool::new(false));
                                    let is_cancelled_clone = is_cancelled.clone();

                                    dialog.on_cancel(move || {
                                        is_cancelled_clone.store(true, Ordering::SeqCst);
                                        // 立即隐藏对话框，无需等待后台线程结束
                                        if let Some(d) = dialog_weak.upgrade() {
                                            let _ = d.hide();
                                        }
                                    });

                                    let _ = dialog.show();
                                    crate::slint_ui::activate_window(&dialog.window());

                                    // 共享状态供后台线程和UI线程通讯 (progress, text, can_cancel, finished)
                                    let progress_state = Arc::new(std::sync::Mutex::new((
                                        0.0f32,
                                        String::new(),
                                        true,
                                        false,
                                    )));

                                    let timer_rc =
                                        Rc::new(RefCell::new(Some(slint::Timer::default())));
                                    let timer_clone = timer_rc.clone();

                                    let progress_state_timer = progress_state.clone();
                                    let is_cancelled_timer = is_cancelled.clone();
                                    let d_weak_timer = dialog.as_weak();

                                    timer_rc.borrow().as_ref().unwrap().start(
                                        slint::TimerMode::Repeated,
                                        std::time::Duration::from_millis(100),
                                        move || {
                                            let mut finished = false;
                                            let is_cancelled_now =
                                                is_cancelled_timer.load(Ordering::SeqCst);

                                            match d_weak_timer.upgrade() {
                                                Some(d) => {
                                                    let (pct, text, can_cancel, is_done) = {
                                                        let state =
                                                            progress_state_timer.lock().unwrap();
                                                        (state.0, state.1.clone(), state.2, state.3)
                                                    };

                                                    // 取消进行中时不覆盖 UI 文字（保持"正在取消..."）
                                                    if !is_cancelled_now && !text.is_empty() {
                                                        d.set_status_text(text.into());
                                                        d.set_progress(pct);
                                                        d.set_can_cancel(can_cancel);
                                                    }

                                                    if is_done {
                                                        finished = true;
                                                        if is_cancelled_now {
                                                            // 取消完成后直接隐藏对话框
                                                            let _ = d.hide();
                                                        }
                                                        // 非取消情况下（成功/失败），对话框保持显示
                                                        // 让用户看到最终状态文字（如"应用更新失败"）
                                                    }
                                                }
                                                _ => {
                                                    finished = true;
                                                }
                                            }

                                            if finished {
                                                if let Ok(mut t) = timer_clone.try_borrow_mut() {
                                                    *t = None;
                                                }
                                            }
                                        },
                                    );

                                    // 3. 在新线程中执行下载及解压
                                    let progress_state_thread = progress_state.clone();
                                    let is_cancelled_thread = is_cancelled.clone();
                                    std::thread::spawn(move || {
                                        let rt2 = tokio::runtime::Builder::new_current_thread()
                                            .enable_all()
                                            .build()
                                            .unwrap();
                                        rt2.block_on(async move {
                                            let extract_dir =
                                                std::env::temp_dir().join("screenhop_update");
                                            let _ = std::fs::remove_dir_all(&extract_dir); // 清理旧缓存
                                            std::fs::create_dir_all(&extract_dir).unwrap();

                                            log::info!("开始下载并解压到: {:?}", extract_dir);

                                            // 定义进度回调
                                            let progress_state_cb = progress_state_thread.clone();
                                            let progress_cb = move |downloaded: u64, total: u64| {
                                                let pct = if total > 0 {
                                                    downloaded as f32 / total as f32
                                                } else {
                                                    0.0
                                                };
                                                let mb_downloaded =
                                                    downloaded as f32 / 1024.0 / 1024.0;
                                                let mb_total = total as f32 / 1024.0 / 1024.0;

                                                let text = format!(
                                                    "正在下载: {:.1} MB / {:.1} MB",
                                                    mb_downloaded, mb_total
                                                );
                                                let mut state = progress_state_cb.lock().unwrap();
                                                state.0 = pct;
                                                state.1 = text;
                                            };

                                            // 执行下载和解压
                                            let dl_res = screenhop::updater::download_and_extract(
                                                &download_url,
                                                &extract_dir,
                                                proxy_url_clone2.as_deref(),
                                                proxy_username_clone2.as_deref(),
                                                proxy_password_clone2.as_deref(),
                                                progress_cb,
                                            )
                                            .await;

                                            if is_cancelled_thread.load(Ordering::SeqCst) {
                                                log::info!("用户已取消更新");
                                                let mut state =
                                                    progress_state_thread.lock().unwrap();
                                                state.3 = true; // finish
                                                return;
                                            }

                                            match dl_res {
                                                Ok(_) => {
                                                    // 4. 下载解压成功，执行应用替换逻辑
                                                    {
                                                        let mut state =
                                                            progress_state_thread.lock().unwrap();
                                                        state.1 =
                                                            "正在安装更新并重启...".to_string();
                                                        state.0 = 1.0;
                                                        state.2 = false;
                                                    }

                                                    // 应用更新
                                                    #[cfg(target_os = "macos")]
                                                    let apply_res =
                                                        screenhop::updater::apply_update_macos(
                                                            &extract_dir,
                                                        );

                                                    #[cfg(target_os = "windows")]
                                                    let apply_res =
                                                        screenhop::updater::apply_update_windows(
                                                            &extract_dir,
                                                        );

                                                    if let Err(e) = apply_res {
                                                        log::error!("应用更新失败: {:?}", e);
                                                        let mut state =
                                                            progress_state_thread.lock().unwrap();
                                                        state.1 = format!("应用更新失败: {}", e);
                                                        state.2 = true;
                                                        state.3 = true;
                                                    } else {
                                                        log::info!(
                                                            "更新应用成功，准备退出当前进程"
                                                        );
                                                        std::process::exit(0);
                                                    }
                                                }
                                                Err(e) => {
                                                    log::error!("下载解压失败: {:?}", e);
                                                    let mut state =
                                                        progress_state_thread.lock().unwrap();
                                                    state.1 = format!("下载失败: {}", e);
                                                    state.2 = true;
                                                    state.3 = true;
                                                }
                                            }
                                        });
                                    });
                                }
                            });
                        } else {
                            log::error!("未找到可用的下载链接");
                            if manual {
                                crate::slint_ui::show_error(
                                    "检查更新",
                                    "未找到适用于当前平台的安装包",
                                );
                            }
                        }
                    } else {
                        log::info!("当前已是最新版本 ({})", result.current_version);
                        if manual {
                            crate::slint_ui::show_alert(
                                "已是最新版本",
                                &format!("当前版本 {} 已是最新。", result.current_version),
                            );
                        }
                    }
                }
                Err(e) => {
                    log::error!("检查更新失败: {}", e);
                    if manual {
                        crate::slint_ui::show_error("检查更新失败", &e.to_string());
                    }
                }
            }
        });
    });
}

struct MenuItems {
    #[cfg(target_os = "macos")]
    status_item: MenuItem,
    toggle_item: MenuItem,
    autostart_item: MenuItem,
    auto_check_update_item: MenuItem,
    proxy_enable_item: MenuItem,
}

fn set_autostart_menu(items: &MenuItems, enabled: bool) {
    items.autostart_item.set_text(if enabled {
        "✓ 开机自动启动"
    } else {
        "  开机自动启动"
    });
}

#[derive(Clone, PartialEq)]
struct SettingsSnapshot {
    dark: bool,
    disable_hook: bool,
    title_bar_height: f64,
    auto_check_update: bool,
    proxy_enabled: bool,
    proxy_url: String,
    proxy_username: Option<String>,
    proxy_password: Option<String>,
    last_update_check_ts: Option<i64>,
}

impl SettingsSnapshot {
    fn from_config(cfg: &AppConfig, dark: bool) -> Self {
        Self {
            dark,
            disable_hook: cfg.disable_hook,
            title_bar_height: cfg.title_bar_height,
            auto_check_update: cfg.auto_check_update,
            proxy_enabled: cfg.proxy_enabled,
            proxy_url: cfg.proxy_url.clone(),
            proxy_username: cfg.proxy_username.clone(),
            proxy_password: cfg.proxy_password.clone(),
            last_update_check_ts: cfg.last_update_check_ts,
        }
    }
}

thread_local! {
    static APPLIED_SETTINGS: RefCell<Option<SettingsSnapshot>> = const { RefCell::new(None) };
    static PROXY_CONFLICT_NOTIFIED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn proxy_has_draft(
    window: &crate::slint_ui::MainWindow,
    applied: Option<&SettingsSnapshot>,
) -> bool {
    let Some(applied) = applied else {
        return false;
    };
    let (protocol, address, port) = parse_proxy_url(&applied.proxy_url);
    window.get_proxy_protocol().as_str() != protocol
        || window.get_proxy_address().as_str() != address
        || window.get_proxy_port().as_str() != port
        || window.get_proxy_auth() != applied.proxy_username.is_some()
        || window.get_proxy_username().as_str()
            != applied.proxy_username.clone().unwrap_or_default()
        || window.get_proxy_password().as_str()
            != applied.proxy_password.clone().unwrap_or_default()
}

fn proxy_changed_externally(
    applied: Option<&SettingsSnapshot>,
    snapshot: &SettingsSnapshot,
) -> bool {
    let Some(applied) = applied else {
        return false;
    };
    applied.proxy_url != snapshot.proxy_url
        || applied.proxy_enabled != snapshot.proxy_enabled
        || applied.proxy_username != snapshot.proxy_username
        || applied.proxy_password != snapshot.proxy_password
}

fn refresh_settings_window(
    window: &crate::slint_ui::MainWindow,
    config: &Arc<Mutex<AppConfig>>,
    dark: bool,
) {
    let total = std::time::Instant::now();
    let lock_started = std::time::Instant::now();
    let cfg = match config.lock() {
        Ok(cfg) => cfg.clone(),
        Err(_) => return,
    };
    let lock_elapsed = lock_started.elapsed();
    let snapshot = SettingsSnapshot::from_config(&cfg, dark);
    let previous = APPLIED_SETTINGS.with(|applied| applied.borrow().clone());
    let full = previous.is_none();
    let stale = |equal: bool| full || !equal;
    let theme_same = previous.as_ref().map_or(false, |p| p.dark == snapshot.dark);
    let hook_same = previous
        .as_ref()
        .map_or(false, |p| p.disable_hook == snapshot.disable_hook);
    let height_same = previous
        .as_ref()
        .map_or(false, |p| p.title_bar_height == snapshot.title_bar_height);
    let auto_check_same = previous
        .as_ref()
        .map_or(false, |p| p.auto_check_update == snapshot.auto_check_update);
    let proxy_enabled_same = previous
        .as_ref()
        .map_or(false, |p| p.proxy_enabled == snapshot.proxy_enabled);
    let proxy_auth_same = previous.as_ref().map_or(false, |p| {
        p.proxy_username.is_some() == snapshot.proxy_username.is_some()
    });
    let proxy_username_same = previous
        .as_ref()
        .map_or(false, |p| p.proxy_username == snapshot.proxy_username);
    let proxy_password_same = previous
        .as_ref()
        .map_or(false, |p| p.proxy_password == snapshot.proxy_password);
    let proxy_url_same = previous
        .as_ref()
        .map_or(false, |p| p.proxy_url == snapshot.proxy_url);
    let last_check_same = previous.as_ref().map_or(false, |p| {
        p.last_update_check_ts == snapshot.last_update_check_ts
    });

    let apply_started = std::time::Instant::now();
    if stale(theme_same) {
        crate::slint_ui::apply_theme_main(window, dark);
    }
    if full {
        window.set_version_text(env!("CARGO_PKG_VERSION").into());
        window.set_app_name(screenhop::app_name().into());
        window.set_dev_build(screenhop::is_dev_build());
        window.set_macos_permissions(cfg!(target_os = "macos"));
        window.set_log_path(std::env::temp_dir().to_string_lossy().to_string().into());
    }
    if stale(hook_same) {
        window.set_hook_enabled(!cfg.disable_hook);
    }
    if stale(height_same) {
        window.set_title_height(cfg.title_bar_height as f32);
        window.set_title_height_text(format!("{} px", cfg.title_bar_height.round() as i32).into());
    }
    if stale(auto_check_same) {
        window.set_auto_check(cfg.auto_check_update);
    }
    let proxy_draft = proxy_has_draft(window, previous.as_ref());
    let proxy_conflict = proxy_draft && proxy_changed_externally(previous.as_ref(), &snapshot);
    if proxy_conflict {
        let first = PROXY_CONFLICT_NOTIFIED.with(|flag| !flag.replace(true));
        if first {
            toast(window, "代理页有未保存修改，已保留；保存后将覆盖外部改动");
        }
        log::info!("代理草稿与外部配置冲突，保留草稿");
    } else {
        PROXY_CONFLICT_NOTIFIED.with(|flag| flag.set(false));
        if stale(proxy_enabled_same) {
            window.set_proxy_enabled(cfg.proxy_enabled);
        }
        if stale(proxy_auth_same) {
            window.set_proxy_auth(cfg.proxy_username.is_some());
        }
        if stale(proxy_username_same) {
            window.set_proxy_username(cfg.proxy_username.clone().unwrap_or_default().into());
        }
        if stale(proxy_password_same) {
            window.set_proxy_password(cfg.proxy_password.clone().unwrap_or_default().into());
        }
        if stale(proxy_url_same) {
            let (protocol, address, port) = parse_proxy_url(&cfg.proxy_url);
            window.set_proxy_protocol(protocol.into());
            window.set_proxy_address(address.into());
            window.set_proxy_port(port.into());
        }
    }
    if stale(last_check_same) {
        window.set_last_check_text(match cfg.last_update_check_ts {
            Some(ts) => format!("上次检查：{}", crate::slint_ui::format_unix_ts(ts)).into(),
            None => "尚未检查更新".into(),
        });
    }
    #[cfg(target_os = "macos")]
    {
        apply_permission_status(window, &crate::macos_runtime::status());
        if let Ok(path) = crate::macos_permissions::application_path() {
            window.set_application_path(path.to_string_lossy().as_ref().into());
        }
    }
    let apply_elapsed = apply_started.elapsed();

    let autostart_started = std::time::Instant::now();
    match current_autostart_status() {
        Ok(status) => {
            window.set_autostart_enabled(status.enabled);
            window.set_autostart_blocked(autostart_blocked(&status));
            window.set_autostart_detail(autostart_detail(&status).into());
        }
        Err(err) => {
            window.set_autostart_blocked(true);
            window.set_autostart_detail(format!("读取开机自启动状态失败：{err}").into());
        }
    }
    let autostart_elapsed = autostart_started.elapsed();

    APPLIED_SETTINGS.with(|applied| *applied.borrow_mut() = Some(snapshot));

    log::info!(
        "设置窗口刷新：{}，锁 {:?}，应用 {:?}，自启状态 {:?}，合计 {:?}",
        if full { "全量" } else { "增量" },
        lock_elapsed,
        apply_elapsed,
        autostart_elapsed,
        total.elapsed()
    );
}

fn sync_settings_window(
    settings: &Rc<RefCell<Option<crate::slint_ui::MainWindow>>>,
    config: &Arc<Mutex<AppConfig>>,
) {
    let slot = settings.borrow();
    let Some(window) = slot.as_ref() else {
        return;
    };
    let dark = config
        .lock()
        .map(|cfg| crate::slint_ui::effective_dark(cfg.theme))
        .unwrap_or(false);
    refresh_settings_window(window, config, dark);
}

fn ensure_system_theme(config: &Arc<Mutex<AppConfig>>) {
    let auto = config
        .lock()
        .map(|cfg| cfg.theme == screenhop::config::Theme::Auto)
        .unwrap_or(false);
    if !auto || crate::slint_ui::cached_system_dark().is_some() {
        return;
    }
    crate::slint_ui::refresh_system_dark(|_| {});
}

#[cfg(target_os = "macos")]
fn bind_permission_controls(
    window: &crate::slint_ui::MainWindow,
    config: &Arc<Mutex<AppConfig>>,
    items: &Rc<MenuItems>,
) {
    let weak = window.as_weak();
    let config_clone = config.clone();
    let items_clone = items.clone();
    window.on_authorize(move || {
        crate::macos_permissions::authorize();
        if let Some(status) = reconcile_permissions(&config_clone, &items_clone, true) {
            if let Some(window) = weak.upgrade() {
                apply_permission_status(&window, &status);
            }
        }
    });
    let weak = window.as_weak();
    let config_clone = config.clone();
    let items_clone = items.clone();
    window.on_recheck_permission(move || {
        if let Some(status) = reconcile_permissions(&config_clone, &items_clone, true) {
            if let Some(window) = weak.upgrade() {
                apply_permission_status(&window, &status);
            }
        }
    });
    window.on_reveal_application(|| {
        if let Err(error) = crate::macos_permissions::reveal_application() {
            crate::slint_ui::show_error("当前应用位置", &format!("{error:#}"));
        }
    });
    window.on_restart_application(|| {
        if let Err(error) = crate::macos_permissions::restart() {
            crate::slint_ui::show_error("重新启动失败", &format!("{error:#}"));
        }
    });
    window.on_permission_help(|| {
        let path = crate::macos_permissions::application_path().map(|path| path.display().to_string()).unwrap_or_else(|_| "未知位置".into());
        crate::slint_ui::show_alert("辅助功能权限", &format!("打开 系统设置 → 隐私与安全性 → 辅助功能，开启 {}。授权后会自动生效，也可点击「重新检测」。如列表中有多个同名应用，请通过「在 Finder 中显示」核对当前副本。\n\n当前位置：{path}\n\n已授权仍启动失败时可重试；系统未向当前进程更新授权时，可重新启动应用。", screenhop::app_name()));
    });
}

fn create_settings_window(
    config: &Arc<Mutex<AppConfig>>,
    items: &Rc<MenuItems>,
    dark: bool,
) -> Option<crate::slint_ui::MainWindow> {
    let window = crate::slint_ui::MainWindow::new().ok()?;
    crate::slint_ui::apply_theme_main(&window, dark);
    crate::slint_ui::bind_main_window(&window);
    refresh_settings_window(&window, config, dark);
    #[cfg(target_os = "macos")]
    bind_permission_controls(&window, config, items);

    let weak = window.as_weak();
    window.on_close_requested(move || {
        APPLIED_SETTINGS.with(|applied| *applied.borrow_mut() = None);
        PROXY_CONFLICT_NOTIFIED.with(|flag| flag.set(false));
        if let Some(window) = weak.upgrade() {
            let _ = window.hide();
        }
    });

    let weak = window.as_weak();
    window.on_minimize_requested(move || {
        if let Some(window) = weak.upgrade() {
            window.window().set_minimized(true);
        }
    });

    let weak = window.as_weak();
    let config_clone = config.clone();
    let items_clone = items.clone();
    window.on_hook_toggled(move |enabled| {
        if let Ok(mut cfg) = config_clone.lock() {
            cfg.disable_hook = !enabled;
            crate::app::set_hook_enabled(enabled);
            items_clone.toggle_item.set_text(if enabled {
                "禁用鼠标中键移动"
            } else {
                "启用鼠标中键移动"
            });
            if let Err(err) = cfg.save() {
                log::error!("保存配置失败: {}", err);
            }
        }
        if let Some(window) = weak.upgrade() {
            #[cfg(target_os = "macos")]
            if let Some(status) = reconcile_permissions(&config_clone, &items_clone, false) {
                apply_permission_status(&window, &status);
            }
            toast(
                &window,
                if enabled {
                    "已开启，将按权限状态启动"
                } else {
                    "已暂停"
                },
            );
        }
    });

    let weak = window.as_weak();
    let config_clone = config.clone();
    window.on_title_height_changed(move |value| {
        if let Ok(mut cfg) = config_clone.lock() {
            cfg.title_bar_height = value as f64;
            if let Err(err) = cfg.save() {
                log::error!("保存配置失败: {}", err);
            }
        }
        if let Some(window) = weak.upgrade() {
            window.set_title_height_text(format!("{} px", value.round() as i32).into());
        }
    });

    let weak = window.as_weak();
    let config_clone = config.clone();
    let items_clone = items.clone();
    window.on_autostart_toggled(move |enabled| {
        if screenhop::is_dev_build() {
            return;
        }
        let auto = screenhop_platform::auto_start();
        match auto.set_enabled(enabled) {
            Ok(status) => {
                set_autostart_menu(&items_clone, status.enabled);
                if let Ok(mut cfg) = config_clone.lock() {
                    cfg.auto_start = status.enabled;
                    if let Err(err) = cfg.save() {
                        log::error!("保存配置失败: {}", err);
                    }
                }
                if let Some(window) = weak.upgrade() {
                    window.set_autostart_enabled(status.enabled);
                    window.set_autostart_blocked(autostart_blocked(&status));
                    window.set_autostart_detail(autostart_detail(&status).into());
                }
                if status.requires_user_action {
                    crate::slint_ui::show_alert("开机自启动", &status.detail);
                }
            }
            Err(err) => {
                log::error!("设置开机自启动失败: {:?}", err);
                crate::slint_ui::show_error("开机自启动", &format!("设置失败：{err}"));
                if let Some(window) = weak.upgrade() {
                    match auto.status() {
                        Ok(status) => {
                            window.set_autostart_enabled(status.enabled);
                            window.set_autostart_blocked(autostart_blocked(&status));
                            window.set_autostart_detail(autostart_detail(&status).into());
                        }
                        Err(_) => window.set_autostart_enabled(!enabled),
                    }
                }
            }
        }
    });

    let weak = window.as_weak();
    let config_clone = config.clone();
    let items_clone = items.clone();
    window.on_fix_autostart(move || {
        if screenhop::is_dev_build() {
            return;
        }
        let auto = screenhop_platform::auto_start();
        match auto.set_enabled(true) {
            Ok(status) => {
                set_autostart_menu(&items_clone, status.enabled);
                if let Ok(mut cfg) = config_clone.lock() {
                    cfg.auto_start = status.enabled;
                    if let Err(err) = cfg.save() {
                        log::error!("保存配置失败: {}", err);
                    }
                }
                if let Some(window) = weak.upgrade() {
                    window.set_autostart_enabled(status.enabled);
                    window.set_autostart_blocked(autostart_blocked(&status));
                    window.set_autostart_detail(autostart_detail(&status).into());
                    toast(&window, "已尝试修复开机自启动");
                }
                if status.requires_user_action {
                    crate::slint_ui::show_alert("开机自启动", &status.detail);
                }
            }
            Err(err) => {
                log::error!("修复开机自启动失败: {:?}", err);
                crate::slint_ui::show_error("开机自启动", &format!("修复失败：{err}"));
            }
        }
    });

    let config_clone = config.clone();
    let items_clone = items.clone();
    window.on_proxy_enabled_toggled(move |enabled| {
        if let Ok(mut cfg) = config_clone.lock() {
            cfg.proxy_enabled = enabled;
            items_clone.proxy_enable_item.set_text(if enabled {
                "✓ 启用代理"
            } else {
                "  启用代理"
            });
            if enabled && cfg.proxy_url.is_empty() {
                cfg.proxy_url = "socks5://127.0.0.1:2888".to_string();
            }
            if let Err(err) = cfg.save() {
                log::error!("保存配置失败: {}", err);
            }
        }
    });

    let weak = window.as_weak();
    let config_clone = config.clone();
    let items_clone = items.clone();
    window.on_save_proxy(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let address = window.get_proxy_address().to_string();
        let port = window.get_proxy_port().to_string();
        let protocol = window.get_proxy_protocol().to_string();
        let auth = window.get_proxy_auth();
        let username = window.get_proxy_username().to_string();
        let password = window.get_proxy_password().to_string();
        let url = format!("{}://{}:{}", protocol_to_scheme(&protocol), address, port);

        let mut overwrote = false;
        if let Ok(mut cfg) = config_clone.lock() {
            overwrote = APPLIED_SETTINGS.with(|applied| {
                applied.borrow().as_ref().map_or(false, |applied| {
                    applied.proxy_url != cfg.proxy_url
                        || applied.proxy_enabled != cfg.proxy_enabled
                        || applied.proxy_username != cfg.proxy_username
                        || applied.proxy_password != cfg.proxy_password
                })
            });
            cfg.proxy_url = url;
            cfg.proxy_enabled = true;
            if auth && !username.is_empty() {
                cfg.proxy_username = Some(username);
                cfg.proxy_password = Some(password);
            } else {
                cfg.proxy_username = None;
                cfg.proxy_password = None;
            }
            items_clone.proxy_enable_item.set_text("✓ 启用代理");
            if let Err(err) = cfg.save() {
                log::error!("保存配置失败: {}", err);
            }
        }
        APPLIED_SETTINGS.with(|applied| *applied.borrow_mut() = None);
        PROXY_CONFLICT_NOTIFIED.with(|flag| flag.set(false));
        window.set_proxy_enabled(true);
        toast(
            &window,
            if overwrote {
                "已覆盖外部改动并保存代理设置"
            } else {
                "代理设置已保存"
            },
        );
    });

    let config_clone = config.clone();
    let items_clone = items.clone();
    window.on_auto_check_toggled(move |enabled| {
        if screenhop::is_dev_build() {
            return;
        }
        if let Ok(mut cfg) = config_clone.lock() {
            cfg.auto_check_update = enabled;
            items_clone.auto_check_update_item.set_text(if enabled {
                "✓ 启动时检查更新"
            } else {
                "  启动时检查更新"
            });
            if let Err(err) = cfg.save() {
                log::error!("保存配置失败: {}", err);
            }
        }
    });

    let weak = window.as_weak();
    let config_clone = config.clone();
    window.on_check_now(move || {
        if screenhop::is_dev_build() {
            return;
        }
        if let Some(window) = weak.upgrade() {
            window.set_check_busy(true);
            toast(&window, "正在检查更新…");
        }
        do_check_update(config_clone.clone(), true, false);
        if let Some(window) = weak.upgrade() {
            window.set_last_check_text(
                format!("上次检查：{}", crate::slint_ui::format_unix_ts(now_unix())).into(),
            );
        }
    });

    window.on_open_repo(move || {
        let _ = open::that(REPO_URL);
    });

    window.on_open_logs(move || {
        let _ = open::that(std::env::temp_dir());
    });

    let _ = window.show();
    crate::slint_ui::activate_window(&window.window());
    Some(window)
}

/// 运行托盘应用主循环
pub fn run_app(config: AppConfig) -> Result<()> {
    // 我们现在使用 slint::run_event_loop_until_quit()，不再需要隐藏窗口来维持事件循环

    let config = Arc::new(Mutex::new(config));

    crate::slint_ui::refresh_system_dark(|_| {});

    // 创建菜单
    let menu = Menu::new();

    let status_item = MenuItem::new(
        format!(
            "{}：{}",
            screenhop::app_name(),
            if cfg!(target_os = "macos") {
                "正在启动"
            } else {
                "已启动"
            }
        ),
        false,
        None,
    );
    #[cfg(target_os = "macos")]
    let permissions_item = MenuItem::with_id(MENU_ID_PERMISSIONS, "权限设置…", true, None);
    let toggle_item = MenuItem::with_id(
        MENU_ID_TOGGLE,
        if config.lock().unwrap().disable_hook {
            "启用鼠标中键移动"
        } else {
            "禁用鼠标中键移动"
        },
        true,
        None,
    );
    let settings_item = MenuItem::with_id(MENU_ID_SETTINGS, "设置...", true, None);
    let autostart_status = match current_autostart_status() {
        Ok(status) => {
            if let Ok(mut cfg) = config.lock() {
                if cfg.auto_start != status.enabled {
                    cfg.auto_start = status.enabled;
                    if let Err(err) = cfg.save() {
                        log::error!("保存配置失败: {}", err);
                    }
                }
            }
            if status.requires_user_action {
                log::warn!("开机自启动需要用户处理: {}", status.detail);
            }
            status
        }
        Err(err) => {
            log::error!("读取开机自启动状态失败: {:?}", err);
            AutoStartStatus::disabled("读取开机自启动状态失败")
        }
    };
    let autostart_item = MenuItem::with_id(
        MENU_ID_AUTOSTART,
        if autostart_status.enabled {
            "✓ 开机自动启动"
        } else {
            "  开机自动启动"
        },
        true,
        None,
    );
    let update_item = MenuItem::with_id(MENU_ID_CHECK_UPDATE, "检查更新", true, None);
    let auto_check_update_item = MenuItem::with_id(
        MENU_ID_AUTO_CHECK_UPDATE,
        if config.lock().unwrap().auto_check_update {
            "✓ 启动时检查更新"
        } else {
            "  启动时检查更新"
        },
        true,
        None,
    );

    let proxy_menu = muda::Submenu::new("代理设置", true);
    let proxy_enable_item = MenuItem::with_id(
        MENU_ID_PROXY_ENABLE,
        {
            let cfg = config.lock().unwrap();
            if cfg.proxy_enabled {
                "✓ 启用代理"
            } else {
                "  启用代理"
            }
        },
        true,
        None,
    );
    let proxy_settings_item = MenuItem::with_id(MENU_ID_PROXY_SETTINGS, "代理设置...", true, None);
    proxy_menu.append(&proxy_enable_item).ok();
    proxy_menu.append(&proxy_settings_item).ok();

    let separator = PredefinedMenuItem::separator();
    let quit_item = MenuItem::with_id(MENU_ID_QUIT, "退出", true, None);

    menu.append(&status_item).ok();
    menu.append(&separator).ok();
    menu.append(&toggle_item).ok();
    menu.append(&settings_item).ok();
    #[cfg(target_os = "macos")]
    menu.append(&permissions_item).ok();
    if screenhop::is_dev_build() {
        autostart_item.set_enabled(false);
        update_item.set_enabled(false);
        auto_check_update_item.set_enabled(false);
    }
    menu.append(&autostart_item).ok();
    menu.append(&update_item).ok();
    menu.append(&auto_check_update_item).ok();
    menu.append(&proxy_menu).ok();
    menu.append(&PredefinedMenuItem::separator()).ok();
    menu.append(&quit_item).ok();

    // 创建托盘图标
    let icon = create_tray_icon_image();
    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip(screenhop::app_name())
        .with_icon(icon)
        .build()?;

    log::info!("系统托盘图标已创建");

    // 启动时自动检查更新（延迟 3s，避开启动竞态；24h 冷却）
    {
        let should_check = match config.lock() {
            Ok(cfg) => should_auto_check(&cfg),
            Err(_) => false,
        };
        if should_check {
            log::info!("启动时自动检查更新...");
            do_check_update(config.clone(), false, true);
        } else {
            log::info!("跳过启动时更新检查（已关闭或处于冷却期）");
        }
    }

    // 在主线程处理 Slint 和 MenuEvents
    let menu_items = Rc::new(MenuItems {
        #[cfg(target_os = "macos")]
        status_item,
        toggle_item,
        autostart_item,
        auto_check_update_item,
        proxy_enable_item,
    });
    let settings_window: Rc<RefCell<Option<crate::slint_ui::MainWindow>>> =
        Rc::new(RefCell::new(None));
    let config_clone = config.clone();
    let menu_items_clone = menu_items.clone();
    let settings_window_clone = settings_window.clone();

    #[cfg(target_os = "macos")]
    let mut macos_policy_set = false;

    #[cfg(target_os = "macos")]
    let permission_timer = {
        let timer = slint::Timer::default();
        let config = config.clone();
        let items = menu_items.clone();
        let settings = settings_window.clone();
        let mut previous = None;
        let mut initialized = false;
        timer.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(250),
            move || {
                if let Some(status) = reconcile_permissions(&config, &items, false) {
                    if !initialized {
                        initialized = true;
                        if status.state == crate::macos_runtime::HookState::NeedsPermission {
                            handle_menu_event(MENU_ID_SETTINGS, &items, &config, &settings);
                        }
                    }
                    if previous.as_ref() != Some(&status) {
                        if let Some(window) = settings.borrow().as_ref() {
                            apply_permission_status(window, &status);
                        }
                        previous = Some(status);
                    }
                }
            },
        );
        timer
    };

    // 定时检查 Menu事件
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        std::time::Duration::from_millis(50),
        move || {
            // macOS: 在事件循环刚启动时将 NSApplication 激活策略设为 Accessory（不在 Dock 显示）
            // 以覆盖 slint/winit 默认强设为 Regular 的行为
            #[cfg(target_os = "macos")]
            if !macos_policy_set {
                #[allow(deprecated)]
                unsafe {
                    use cocoa::appkit::NSApp;
                    use objc::*;
                    let app = NSApp();
                    let _: () = msg_send![app, setActivationPolicy: 1i64]; // Accessory

                    // Set the application icon
                    let icon_bytes = include_bytes!("../assets/icon-256.png");
                    let data = cocoa::foundation::NSData::dataWithBytes_length_(
                        cocoa::base::nil,
                        icon_bytes.as_ptr() as *const std::ffi::c_void,
                        icon_bytes.len() as u64,
                    );
                    let ns_image: cocoa::base::id = msg_send![class!(NSImage), alloc];
                    let ns_image: cocoa::base::id = msg_send![ns_image, initWithData: data];
                    if ns_image != cocoa::base::nil {
                        let _: () = msg_send![app, setApplicationIconImage: ns_image];
                    }

                    // 强制解除此后台应用对前台的抢占，把焦点还给此前的应用
                    // 修复首次点出托盘菜单时当前工作窗口突然失去焦点的 Bug
                    // (winit在 cargo run 期间会强制把我们变成 Regular，所以这步补救是必须的)
                    let _: () = msg_send![app, deactivate];
                }
                macos_policy_set = true;
            }

            if crate::slint_ui::take_theme_refresh() {
                APPLIED_SETTINGS.with(|applied| *applied.borrow_mut() = None);
                sync_settings_window(&settings_window_clone, &config_clone);
            }

            while let Ok(event) = MenuEvent::receiver().try_recv() {
                if event.id.0 == MENU_ID_QUIT {
                    let _ = slint::quit_event_loop();
                    return;
                }
                handle_menu_event(
                    &event.id.0,
                    &menu_items_clone,
                    &config_clone,
                    &settings_window_clone,
                );
            }
        },
    );
    let result = slint::run_event_loop_until_quit();
    timer.stop();
    #[cfg(target_os = "macos")]
    {
        permission_timer.stop();
        crate::macos_runtime::shutdown();
    }
    result?;
    Ok(())
}

fn handle_menu_event(
    id: &str,
    items: &Rc<MenuItems>,
    config: &Arc<Mutex<AppConfig>>,
    settings: &Rc<RefCell<Option<crate::slint_ui::MainWindow>>>,
) {
    match id {
        MENU_ID_TOGGLE => {
            if let Ok(mut cfg) = config.lock() {
                cfg.disable_hook = !cfg.disable_hook;
                let state = if cfg.disable_hook {
                    "已禁用"
                } else {
                    "已启用"
                };
                log::info!("鼠标中键移动功能{}", state);

                let text = if cfg.disable_hook {
                    "启用鼠标中键移动"
                } else {
                    "禁用鼠标中键移动"
                };
                items.toggle_item.set_text(text);

                // 实时启用/禁用钩子（无需重启）
                crate::app::set_hook_enabled(!cfg.disable_hook);

                if let Err(e) = cfg.save() {
                    log::error!("保存配置失败: {}", e);
                }
            }
            #[cfg(target_os = "macos")]
            let _ = reconcile_permissions(config, items, false);
            sync_settings_window(settings, config);
        }
        #[cfg(target_os = "macos")]
        MENU_ID_PERMISSIONS => {
            handle_menu_event(MENU_ID_SETTINGS, items, config, settings);
            if let Some(window) = settings.borrow().as_ref() {
                window.set_page(0);
            }
        }
        MENU_ID_SETTINGS => {
            #[cfg(target_os = "macos")]
            let _ = reconcile_permissions(config, items, true);
            let started = std::time::Instant::now();
            let dark = config
                .lock()
                .map(|cfg| crate::slint_ui::effective_dark(cfg.theme))
                .unwrap_or(false);
            log::info!("设置菜单主题解析耗时 {:?}", started.elapsed());
            let mut slot = settings.borrow_mut();
            match slot.as_ref() {
                Some(window) => {
                    refresh_settings_window(window, config, dark);
                    let _ = window.show();
                    crate::slint_ui::activate_window(&window.window());
                }
                None => {
                    if let Some(window) = create_settings_window(config, items, dark) {
                        *slot = Some(window);
                    }
                }
            }
            drop(slot);
            ensure_system_theme(config);
            log::info!("设置菜单处理耗时 {:?}", started.elapsed());
        }
        MENU_ID_AUTOSTART => {
            if screenhop::is_dev_build() {
                return;
            }
            let desired = match config.lock() {
                Ok(cfg) => !cfg.auto_start,
                Err(_) => false,
            };
            let auto = screenhop_platform::auto_start();
            match auto.set_enabled(desired) {
                Ok(status) => {
                    log::info!("开机自启动: {} ({})", status.enabled, status.detail);
                    set_autostart_menu(items, status.enabled);
                    if let Ok(mut cfg) = config.lock() {
                        cfg.auto_start = status.enabled;
                        if let Err(err) = cfg.save() {
                            log::error!("保存配置失败: {}", err);
                        }
                    }
                    if status.requires_user_action {
                        crate::slint_ui::show_alert("开机自启动", &status.detail);
                    }
                }
                Err(err) => {
                    log::error!("设置开机自启动失败: {:?}", err);
                    crate::slint_ui::show_error("设置开机自启动失败", &format!("{err}"));
                }
            }
            sync_settings_window(settings, config);
        }

        MENU_ID_CHECK_UPDATE => {
            log::info!("手动检查更新...");
            do_check_update(config.clone(), true, false);
        }
        MENU_ID_AUTO_CHECK_UPDATE => {
            if screenhop::is_dev_build() {
                return;
            }
            if let Ok(mut cfg) = config.lock() {
                cfg.auto_check_update = !cfg.auto_check_update;
                log::info!("启动时检查更新: {}", cfg.auto_check_update);

                let text = if cfg.auto_check_update {
                    "✓ 启动时检查更新"
                } else {
                    "  启动时检查更新"
                };
                items.auto_check_update_item.set_text(text);

                if let Err(e) = cfg.save() {
                    log::error!("保存配置失败: {}", e);
                }
            }
            sync_settings_window(settings, config);
        }
        MENU_ID_PROXY_ENABLE => {
            if let Ok(mut cfg) = config.lock() {
                if cfg.proxy_enabled {
                    cfg.proxy_enabled = false;
                    log::info!("代理已禁用");
                    items.proxy_enable_item.set_text("  启用代理");
                    if let Err(e) = cfg.save() {
                        log::error!("保存配置失败: {}", e);
                    }
                } else {
                    if cfg.proxy_url.is_empty() {
                        cfg.proxy_url = "socks5://127.0.0.1:2888".to_string();
                        log::info!("未配置代理地址，自动填充为默认: {}", cfg.proxy_url);
                    }
                    cfg.proxy_enabled = true;
                    log::info!("代理已启用");
                    items.proxy_enable_item.set_text("✓ 启用代理");
                    if let Err(e) = cfg.save() {
                        log::error!("保存配置失败: {}", e);
                    }
                }
            }
            sync_settings_window(settings, config);
        }
        MENU_ID_PROXY_SETTINGS => {
            let (mut address, mut port, mut protocol, mut username, mut password, mut enabled) = (
                "127.0.0.1".to_string(),
                "2888".to_string(),
                "SOCKS5".to_string(),
                String::new(),
                String::new(),
                false,
            );

            if let Ok(cfg) = config.lock() {
                enabled = cfg.proxy_username.is_some();
                username = cfg.proxy_username.clone().unwrap_or_default();
                password = cfg.proxy_password.clone().unwrap_or_default();

                if !cfg.proxy_url.is_empty() {
                    let url = cfg.proxy_url.as_str();
                    let (scheme, rest): (&str, &str) =
                        url.split_once("://").unwrap_or(("socks5", url));

                    let scheme = scheme.to_ascii_lowercase();
                    protocol = match scheme.as_str() {
                        "socks4" | "socks4a" => "SOCKS4",
                        "https" => "HTTPS",
                        "http" => "HTTP",
                        _ => "SOCKS5",
                    }
                    .to_string();

                    if let Some((addr, p)) = rest.split_once(':') {
                        address = addr.to_string();
                        port = p.to_string();
                    } else {
                        address = rest.to_string();
                    }
                }
            }

            let dialog = crate::slint_ui::ProxyAuthDialog::new().unwrap();
            let dark = config
                .lock()
                .map(|cfg| crate::slint_ui::effective_dark(cfg.theme))
                .unwrap_or(false);
            crate::slint_ui::apply_theme_proxy(&dialog, dark);

            dialog.set_address(address.into());
            dialog.set_port(port.into());
            dialog.set_protocol(protocol.into());
            dialog.set_auth_enabled(enabled);
            dialog.set_username(username.into());
            dialog.set_password(password.into());

            let dialog_weak2 = dialog.as_weak();
            dialog.on_cancel(move || {
                if let Some(d) = dialog_weak2.upgrade() {
                    let _ = d.hide();
                }
            });

            let dialog_weak = dialog.as_weak();
            let config_clone = config.clone();
            let proxy_enable_item_clone = items.proxy_enable_item.clone();

            dialog.on_apply(
                move |addr,
                      p,
                      proto: slint::SharedString,
                      auth_en: bool,
                      user: slint::SharedString,
                      pass: slint::SharedString| {
                    let proxy_url_str = match proto.as_str() {
                        "SOCKS5" => format!("socks5://{}:{}", addr, p),
                        "SOCKS4" => format!("socks4://{}:{}", addr, p),
                        "HTTPS" => format!("https://{}:{}", addr, p),
                        "HTTP" => format!("http://{}:{}", addr, p),
                        _ => format!("socks5://{}:{}", addr, p),
                    };

                    if let Ok(mut cfg) = config_clone.lock() {
                        cfg.proxy_url = proxy_url_str.clone();
                        cfg.proxy_enabled = true;
                        if auth_en && !user.is_empty() {
                            cfg.proxy_username = Some(user.into());
                            cfg.proxy_password = Some(pass.into());
                        } else {
                            cfg.proxy_username = None;
                            cfg.proxy_password = None;
                        }

                        log::info!("代理地址及认证已配置: {}", cfg.proxy_url);
                        proxy_enable_item_clone.set_text("✓ 启用代理");

                        if let Err(e) = cfg.save() {
                            log::error!("保存配置失败: {}", e);
                        }
                    }
                    if let Some(d) = dialog_weak.upgrade() {
                        let _ = d.hide();
                    }
                },
            );

            let _ = dialog.show();
            crate::slint_ui::activate_window(&dialog.window());
        }
        MENU_ID_QUIT => {
            log::info!("用户请求退出");
            std::process::exit(0);
        }
        _ => {}
    }
}
