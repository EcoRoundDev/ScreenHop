use crate::{AutoStart, AutoStartStatus, BlockReason};
use anyhow::{anyhow, bail, Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const AGENT_LABEL: &str = "com.dongdong.screenhop";

pub struct MacAutoStart;

impl Default for MacAutoStart {
    fn default() -> Self {
        Self::new()
    }
}

impl MacAutoStart {
    pub fn new() -> Self {
        Self
    }

    fn launch_agent_path() -> Option<PathBuf> {
        let home = std::env::var_os("HOME")?;
        Some(
            PathBuf::from(home)
                .join("Library/LaunchAgents")
                .join(format!("{AGENT_LABEL}.plist")),
        )
    }

    fn launch_agent_status(&self, exe: &Path) -> AutoStartStatus {
        let Some(plist) = Self::launch_agent_path() else {
            return AutoStartStatus::blocked(
                BlockReason::Unsupported,
                false,
                "无法定位 LaunchAgents 目录",
            );
        };
        if !plist.exists() {
            return AutoStartStatus::disabled("未注册登录项");
        }
        match fs::read_to_string(&plist) {
            Ok(content) => {
                if !content.contains(&exe.to_string_lossy().to_string()) {
                    return AutoStartStatus::blocked(
                        BlockReason::TargetMissing {
                            path: plist.display().to_string(),
                        },
                        false,
                        "登录项指向旧版本程序",
                    );
                }
                if agent_loaded() {
                    AutoStartStatus::enabled("登录项已启用")
                } else {
                    AutoStartStatus::blocked(
                        BlockReason::NeedsApproval,
                        false,
                        "登录项文件存在但未加载",
                    )
                }
            }
            Err(err) => AutoStartStatus::blocked(
                BlockReason::PermissionDenied,
                false,
                format!("无法读取登录项文件: {err}"),
            ),
        }
    }
}

impl AutoStart for MacAutoStart {
    fn status(&self) -> Result<AutoStartStatus> {
        let exe = std::env::current_exe().context("无法获取当前程序路径")?;
        match sm_app_service_status() {
            Some(code) => Ok(match code {
                1 => AutoStartStatus::enabled("登录项已启用"),
                2 => AutoStartStatus::blocked(
                    BlockReason::NeedsApproval,
                    true,
                    "登录项需要用户在「系统设置 → 通用 → 登录项与扩展」中批准后才生效",
                ),
                3 => AutoStartStatus::blocked(
                    BlockReason::NeedsApproval,
                    false,
                    "系统未找到该登录项，请先把 ScreenHop 放入「应用程序」文件夹",
                ),
                _ => AutoStartStatus::disabled("未注册登录项"),
            }),
            None => Ok(self.launch_agent_status(&exe)),
        }
    }

    fn set_enabled(&self, enabled: bool) -> Result<AutoStartStatus> {
        let exe = std::env::current_exe().context("无法获取当前程序路径")?;

        if sm_app_service_available() {
            if enabled {
                register_app_service()?;
                let status = self.status()?;
                if status.requires_user_action {
                    open_login_items_settings();
                    log::info!("登录项需要用户批准，已打开系统设置");
                }
                Ok(status)
            } else {
                unregister_app_service()?;
                self.status()
            }
        } else {
            let plist = Self::launch_agent_path()
                .ok_or_else(|| anyhow!("无法定位 LaunchAgents 目录"))?;
            if enabled {
                write_launch_agent(&plist, &exe)?;
                run_launchctl(&["load", "-w", &plist.to_string_lossy()])?;
            } else {
                let _ = run_launchctl(&["unload", "-w", &plist.to_string_lossy()]);
                if plist.exists() {
                    fs::remove_file(&plist).context("删除登录项文件失败")?;
                }
            }
            self.status()
        }
    }
}

fn sm_app_service_available() -> bool {
    objc::runtime::Class::get("SMAppService").is_some()
}

fn sm_app_service_status() -> Option<i64> {
    unsafe {
        use objc::*;
        let cls = objc::runtime::Class::get("SMAppService")?;
        let service: *mut objc::runtime::Object = msg_send![cls, mainAppService];
        if service.is_null() {
            return None;
        }
        let status: i64 = msg_send![service, status];
        Some(status)
    }
}

fn register_app_service() -> Result<()> {
    unsafe {
        use objc::*;
        let cls = objc::runtime::Class::get("SMAppService")
            .ok_or_else(|| anyhow!("当前系统不支持 SMAppService"))?;
        let service: *mut objc::runtime::Object = msg_send![cls, mainAppService];
        if service.is_null() {
            bail!("无法获取主应用登录项");
        }
        let mut error: *mut objc::runtime::Object = std::ptr::null_mut();
        let ok: i8 = msg_send![service, registerAndReturnError: &mut error];
        if ok == 0 {
            bail!("注册登录项失败: {}", ns_error_message(error));
        }
        Ok(())
    }
}

fn unregister_app_service() -> Result<()> {
    unsafe {
        use objc::*;
        let cls = objc::runtime::Class::get("SMAppService")
            .ok_or_else(|| anyhow!("当前系统不支持 SMAppService"))?;
        let service: *mut objc::runtime::Object = msg_send![cls, mainAppService];
        if service.is_null() {
            bail!("无法获取主应用登录项");
        }
        let mut error: *mut objc::runtime::Object = std::ptr::null_mut();
        let ok: i8 = msg_send![service, unregisterAndReturnError: &mut error];
        if ok == 0 {
            bail!("移除登录项失败: {}", ns_error_message(error));
        }
        Ok(())
    }
}

unsafe fn ns_error_message(error: *mut objc::runtime::Object) -> String {
    use objc::*;
    if error.is_null() {
        return "未知错误".to_string();
    }
    let description: *mut objc::runtime::Object = msg_send![error, localizedDescription];
    if description.is_null() {
        return "未知错误".to_string();
    }
    let utf8: *const std::os::raw::c_char = msg_send![description, UTF8String];
    if utf8.is_null() {
        return "未知错误".to_string();
    }
    std::ffi::CStr::from_ptr(utf8)
        .to_string_lossy()
        .into_owned()
}

fn open_login_items_settings() {
    let _ = Command::new("open")
        .arg("x-apple.systempreferences:com.apple.LoginItems-Settings.extension")
        .spawn();
}

fn agent_loaded() -> bool {
    Command::new("launchctl")
        .args(["list", AGENT_LABEL])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn run_launchctl(args: &[&str]) -> Result<()> {
    let output = Command::new("launchctl")
        .args(args)
        .output()
        .context("无法执行 launchctl")?;
    if !output.status.success() {
        bail!(
            "launchctl {} 失败: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(())
}

fn write_launch_agent(plist: &Path, exe: &Path) -> Result<()> {
    if let Some(parent) = plist.parent() {
        fs::create_dir_all(parent).context("创建 LaunchAgents 目录失败")?;
    }
    let exe = escape_xml(&exe.to_string_lossy());
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{AGENT_LABEL}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{exe}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#
    );
    fs::write(plist, content).context("写入登录项文件失败")
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
