use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn application_path() -> Result<PathBuf> {
    let executable = std::env::current_exe().context("无法获取当前应用位置")?;
    Ok(bundle_path(&executable).unwrap_or(executable))
}

fn bundle_path(executable: &Path) -> Option<PathBuf> {
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && bundle.extension()? == "app")
        .then(|| bundle.to_path_buf())
}

pub fn authorize() {
    screenhop_platform::create_platform().request_accessibility_permissions();
    if let Err(err) =
        open::that("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
    {
        log::warn!("打开辅助功能设置失败: {err}");
        crate::slint_ui::show_alert("前往授权", "请打开 系统设置 → 隐私与安全性 → 辅助功能，开启当前运行的 ScreenHop。授权后将自动生效。");
    }
}

pub fn reveal_application() -> Result<()> {
    let path = application_path()?;
    Command::new("/usr/bin/open")
        .arg("-R")
        .arg(path)
        .spawn()
        .context("无法在 Finder 中显示当前应用")?;
    Ok(())
}

pub fn restart() -> Result<()> {
    let executable = std::env::current_exe().context("无法获取当前可执行文件")?;
    Command::new(executable)
        .arg("--screenhop-relaunch-after")
        .arg(std::process::id().to_string())
        .arg(application_path()?)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("无法启动重启辅助进程")?;
    slint::quit_event_loop().context("无法退出当前事件循环")
}

pub fn run_relaunch_helper() -> bool {
    let args: Vec<_> = std::env::args_os().collect();
    if args
        .get(1)
        .is_none_or(|arg| arg != "--screenhop-relaunch-after")
    {
        return false;
    }
    if args.len() != 4 {
        return true;
    }
    let Some(pid) = args[2]
        .to_str()
        .and_then(|s| s.parse::<i32>().ok())
        .filter(|pid| *pid > 1)
    else {
        return true;
    };
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    for _ in 0..300 {
        if unsafe { kill(pid, 0) } != 0 {
            let path = PathBuf::from(&args[3]);
            if path.extension().is_some_and(|ext| ext == "app") {
                let _ = Command::new("/usr/bin/open").arg("-n").arg(path).status();
            } else {
                let _ = Command::new(path).spawn();
            }
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_only_a_standard_bundle_layout() {
        assert_eq!(
            bundle_path(Path::new(
                "/Users/test/Applications/ScreenHop Dev.app/Contents/MacOS/screenhop"
            )),
            Some(PathBuf::from("/Users/test/Applications/ScreenHop Dev.app"))
        );
        assert_eq!(bundle_path(Path::new("/tmp/screenhop")), None);
        assert_eq!(
            bundle_path(Path::new("/tmp/fake.app/Other/MacOS/screenhop")),
            None
        );
    }
}
