#![allow(unexpected_cfgs)]
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
#[cfg(target_os = "macos")]
mod macos_permissions;
#[cfg(target_os = "macos")]
mod macos_runtime;
mod single_instance;
mod slint_ui;
mod tray;

use anyhow::{Context, Result};
use screenhop::config::AppConfig;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

fn inner_main() -> Result<()> {
    // 单实例检测
    let Some(_instance_lock) =
        single_instance::try_lock(screenhop::app_id()).context("单实例检测失败")?
    else {
        log::warn!("程序已在运行中，退出");
        return Ok(());
    };

    // 加载配置
    let mut config = AppConfig::load().context("加载配置失败")?;
    log::info!("配置已加载: {:?}", config);

    if !screenhop::is_dev_build() {
        use screenhop_platform::AutoStart;
        match screenhop_platform::auto_start().reconcile(config.auto_start) {
            Ok(status) => {
                log::info!("开机自启动状态: {}", status.detail);
                if status.requires_user_action {
                    log::warn!("开机自启动需要用户处理: {}", status.detail);
                }
                if config.auto_start != status.enabled {
                    config.auto_start = status.enabled;
                    if let Err(err) = config.save() {
                        log::error!("保存配置失败: {}", err);
                    }
                }
            }
            Err(err) => log::error!("同步开机自启动失败: {:?}", err),
        }
    }

    #[cfg(target_os = "macos")]
    {
        tray::run_app(config)?;
        macos_runtime::shutdown();
    }

    // Windows: 安装钩子 + 启动托盘 + 消息循环
    #[cfg(target_os = "windows")]
    {
        // 安装鼠标中键事件钩子
        if !config.disable_hook {
            app::install_hook(&config)?;
        }

        // 启动系统托盘 + Windows 消息循环（阻塞）
        tray::run_app(config)?;
    }

    log::info!("ScreenHop 已退出");
    Ok(())
}

fn main() {
    #[cfg(target_os = "macos")]
    if macos_permissions::run_relaunch_helper() {
        return;
    }
    // 获取系统的临时目录
    let temp_dir = std::env::temp_dir();
    let run_log_path = temp_dir.join(if screenhop::is_dev_build() {
        "screenhop_dev_run.log"
    } else {
        "screenhop_run.log"
    });
    let err_log_path = temp_dir.join(if screenhop::is_dev_build() {
        "screenhop_dev_fatal_err.log"
    } else {
        "screenhop_fatal_err.log"
    });

    // 初始化日志到文件，方便 Finder 启动时或隐藏执行时调试
    let log_file = std::fs::File::create(&run_log_path).unwrap();
    let target = Box::new(log_file);
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_millis()
        .target(env_logger::Target::Pipe(target))
        .init();

    log::info!("ScreenHop v{} 启动中...", APP_VERSION);

    // 注册 panic hook：捕获任意线程的 panic 并写入错误日志
    // 在 Windows 无控制台模式下，这是唯一能感知后台崩溃的途径
    let err_log_path_clone = err_log_path.clone();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown".to_string());
        let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic payload".to_string()
        };
        let msg = format!("[PANIC] at {}: {}", location, payload);
        log::error!("{}", msg);
        // 同时写入独立的错误文件，方便排查
        let _ = std::fs::write(&err_log_path_clone, &msg);
    }));

    if let Err(e) = inner_main() {
        log::error!("致命错误导致应用退出: {:?}", e);
        std::fs::write(&err_log_path, format!("{:?}", e)).ok();
    }
}
