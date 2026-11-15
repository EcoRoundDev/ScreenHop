use anyhow::{anyhow, bail, Context, Result};
use std::path::Path;
use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegGetValueW, RegSetValueExW, HKEY, REG_SAM_FLAGS,
    HKEY_CURRENT_USER,
    KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_ROUTINE_FLAGS, REG_SZ,
    REG_VALUE_TYPE, RRF_RT_REG_BINARY, RRF_RT_REG_SZ,
};

use crate::{AutoStart, AutoStartStatus, BlockReason};

const VALUE_NAME: &str = "ScreenHop";
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const APPROVED_KEY: &str =
    "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\StartupApproved\\Run";

pub struct WinAutoStart {
    backend: Box<dyn RegistryBackend>,
}

impl Default for WinAutoStart {
    fn default() -> Self {
        Self::new()
    }
}

impl WinAutoStart {
    pub fn new() -> Self {
        Self {
            backend: Box::new(WinRegistry),
        }
    }
}

impl AutoStart for WinAutoStart {
    fn status(&self) -> Result<AutoStartStatus> {
        let exe = std::env::current_exe().context("无法获取当前程序路径")?;
        let run = self.backend.read_string(RUN_KEY, VALUE_NAME)?;
        let approved = self.backend.read_binary(APPROVED_KEY, VALUE_NAME)?;
        Ok(evaluate(run.as_deref(), approved.as_deref(), &exe))
    }

    fn set_enabled(&self, enabled: bool) -> Result<AutoStartStatus> {
        let exe = std::env::current_exe().context("无法获取当前程序路径")?;
        if enabled {
            let quoted = format!("\"{}\"", exe.display());
            self.backend.write_string(RUN_KEY, VALUE_NAME, &quoted)?;
            self.backend.delete(APPROVED_KEY, VALUE_NAME)?;
            let status = self.status()?;
            if !status.enabled {
                bail!("写入开机启动项后校验失败：{}", status.detail);
            }
            log::info!("已启用开机自启动: {}", exe.display());
            Ok(status)
        } else {
            self.backend.delete(RUN_KEY, VALUE_NAME)?;
            self.backend.delete(APPROVED_KEY, VALUE_NAME)?;
            log::info!("已关闭开机自启动");
            Ok(AutoStartStatus::disabled("开机自启动已关闭"))
        }
    }
}

fn parse_registered_path(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(rest) = value.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some(rest[..end].to_string());
    }
    Some(value.to_string())
}

fn same_path(registered: &str, exe: &Path) -> bool {
    let current = exe.to_string_lossy();
    registered.trim().eq_ignore_ascii_case(current.as_ref())
}

fn approved_disabled(approved: Option<&[u8]>) -> Option<Option<String>> {
    let bytes = approved?;
    if !bytes.first().is_some_and(|byte| byte & 1 == 1) {
        return None;
    }
    let filetime = if bytes.len() >= 12 {
        u64::from_le_bytes([
            bytes[4], bytes[5], bytes[6], bytes[7], bytes[8], bytes[9], bytes[10], bytes[11],
        ])
    } else {
        0
    };
    Some(format_filetime(filetime))
}

fn evaluate(run: Option<&str>, approved: Option<&[u8]>, exe: &Path) -> AutoStartStatus {
    let Some(value) = run else {
        return AutoStartStatus::disabled("未注册开机启动项");
    };
    let Some(registered) = parse_registered_path(value) else {
        return AutoStartStatus::disabled("开机启动项内容为空");
    };
    if !same_path(&registered, exe) {
        return AutoStartStatus::blocked(
            BlockReason::TargetMissing {
                path: registered.clone(),
            },
            false,
            format!(
                "开机启动项指向 {}，与当前程序 {} 不一致",
                registered,
                exe.display()
            ),
        );
    }
    if let Some(since) = approved_disabled(approved) {
        let detail = match &since {
            Some(time) => format!("开机自启动已被系统禁用（{}）", time),
            None => "开机自启动已被系统禁用".to_string(),
        };
        return AutoStartStatus::blocked(BlockReason::DisabledBySystem { since }, true, detail);
    }
    AutoStartStatus::enabled("开机自启动已启用")
}

fn format_filetime(filetime: u64) -> Option<String> {
    const EPOCH_DIFF: u64 = 116_444_736_000_000_000;
    if filetime == 0 || filetime < EPOCH_DIFF {
        return None;
    }
    let seconds = (filetime - EPOCH_DIFF) / 10_000_000;
    let days = (seconds / 86_400) as i64;
    let remainder = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02} UTC",
        year,
        month,
        day,
        remainder / 3600,
        (remainder % 3600) / 60
    ))
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

trait RegistryBackend {
    fn read_string(&self, key: &str, name: &str) -> Result<Option<String>>;
    fn read_binary(&self, key: &str, name: &str) -> Result<Option<Vec<u8>>>;
    fn write_string(&self, key: &str, name: &str, value: &str) -> Result<()>;
    fn delete(&self, key: &str, name: &str) -> Result<()>;
}

struct WinRegistry;

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

impl WinRegistry {
    fn open(key: &str, access: REG_SAM_FLAGS) -> Result<HKEY> {
        let key_w = wide(key);
        let mut hkey = HKEY::default();
        unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(key_w.as_ptr()),
                0,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                access,
                None,
                &mut hkey,
                None,
            )
        }
        .ok()
        .map_err(|err| anyhow!("打开注册表项 {} 失败: {:?}", key, err))?;
        Ok(hkey)
    }

    fn query(hkey: HKEY, name: &str, flags: REG_ROUTINE_FLAGS) -> Result<Option<(REG_VALUE_TYPE, Vec<u8>)>> {
        let name_w = wide(name);
        let mut buffer = vec![0u8; 32 * 1024];
        let mut size = buffer.len() as u32;
        let mut kind = REG_VALUE_TYPE::default();
        let result = unsafe {
            RegGetValueW(
                hkey,
                None,
                PCWSTR(name_w.as_ptr()),
                flags,
                Some(&mut kind),
                Some(buffer.as_mut_ptr() as *mut _),
                Some(&mut size),
            )
        };
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        result
            .ok()
            .map_err(|err| anyhow!("读取注册表值 {} 失败: {:?}", name, err))?;
        buffer.truncate(size as usize);
        Ok(Some((kind, buffer)))
    }
}

impl RegistryBackend for WinRegistry {
    fn read_string(&self, key: &str, name: &str) -> Result<Option<String>> {
        let hkey = Self::open(key, KEY_QUERY_VALUE)?;
        let result = Self::query(hkey, name, RRF_RT_REG_SZ);
        let _ = unsafe { RegCloseKey(hkey) };
        let Some((kind, bytes)) = result? else {
            return Ok(None);
        };
        if kind != REG_SZ {
            return Ok(None);
        }
        let units: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let text = String::from_utf16_lossy(&units);
        Ok(Some(text.trim_end_matches('\0').to_string()))
    }

    fn read_binary(&self, key: &str, name: &str) -> Result<Option<Vec<u8>>> {
        let hkey = Self::open(key, KEY_QUERY_VALUE)?;
        let result = Self::query(hkey, name, RRF_RT_REG_BINARY);
        let _ = unsafe { RegCloseKey(hkey) };
        Ok(result?.map(|(_, bytes)| bytes))
    }

    fn write_string(&self, key: &str, name: &str, value: &str) -> Result<()> {
        let hkey = Self::open(key, KEY_QUERY_VALUE | KEY_SET_VALUE)?;
        let name_w = wide(name);
        let mut bytes: Vec<u8> = value
            .encode_utf16()
            .flat_map(|unit| unit.to_le_bytes())
            .collect();
        bytes.extend_from_slice(&[0, 0]);
        let result = unsafe {
            RegSetValueExW(hkey, PCWSTR(name_w.as_ptr()), 0, REG_SZ, Some(&bytes))
        };
        let _ = unsafe { RegCloseKey(hkey) };
        result
            .ok()
            .map_err(|err| anyhow!("写入注册表值 {} 失败: {:?}", name, err))
    }

    fn delete(&self, key: &str, name: &str) -> Result<()> {
        let hkey = Self::open(key, KEY_QUERY_VALUE | KEY_SET_VALUE)?;
        let name_w = wide(name);
        let result = unsafe { RegDeleteValueW(hkey, PCWSTR(name_w.as_ptr())) };
        let _ = unsafe { RegCloseKey(hkey) };
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        result
            .ok()
            .map_err(|err| anyhow!("删除注册表值 {} 失败: {:?}", name, err))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Mutex;

    #[derive(Default)]
    struct MemoryRegistry {
        data: Mutex<HashMap<(String, String), Vec<u8>>>,
    }

    impl RegistryBackend for MemoryRegistry {
        fn read_string(&self, key: &str, name: &str) -> Result<Option<String>> {
            let data = self.data.lock().unwrap();
            Ok(data
                .get(&(key.to_string(), name.to_string()))
                .map(|bytes| String::from_utf8_lossy(bytes).into_owned()))
        }

        fn read_binary(&self, key: &str, name: &str) -> Result<Option<Vec<u8>>> {
            let data = self.data.lock().unwrap();
            Ok(data
                .get(&(key.to_string(), name.to_string()))
                .cloned())
        }

        fn write_string(&self, key: &str, name: &str, value: &str) -> Result<()> {
            self.data
                .lock()
                .unwrap()
                .insert((key.to_string(), name.to_string()), value.as_bytes().to_vec());
            Ok(())
        }

        fn delete(&self, key: &str, name: &str) -> Result<()> {
            self.data
                .lock()
                .unwrap()
                .remove(&(key.to_string(), name.to_string()));
            Ok(())
        }
    }

    fn exe() -> PathBuf {
        PathBuf::from("C:\\Apps\\ScreenHop\\screenhop.exe")
    }

    #[test]
    fn enabled_when_registered_and_not_blocked() {
        let status = evaluate(Some("\"C:\\Apps\\ScreenHop\\screenhop.exe\""), None, &exe());
        assert!(status.enabled);
        assert!(status.blocked.is_none());
    }

    #[test]
    fn disabled_when_startup_approved_blocked() {
        let mut approved = vec![3, 0, 0, 0];
        approved.extend_from_slice(&0u64.to_le_bytes());
        let status = evaluate(Some("\"C:\\Apps\\ScreenHop\\screenhop.exe\""), Some(&approved), &exe());
        assert!(!status.enabled);
        assert!(status.requires_user_action);
        assert!(matches!(
            status.blocked,
            Some(BlockReason::DisabledBySystem { .. })
        ));
    }

    #[test]
    fn blocked_when_registered_path_is_stale() {
        let status = evaluate(Some("\"D:\\old\\screenhop.exe\""), None, &exe());
        assert!(!status.enabled);
        assert!(matches!(
            status.blocked,
            Some(BlockReason::TargetMissing { .. })
        ));
    }

    #[test]
    fn disabled_without_registry_value() {
        let status = evaluate(None, None, &exe());
        assert!(!status.enabled);
        assert!(status.blocked.is_none());
    }

    #[test]
    fn set_enabled_roundtrip_with_memory_backend() {
        let autostart = WinAutoStart {
            backend: Box::new(MemoryRegistry::default()),
        };
        let status = autostart.set_enabled(true).unwrap();
        assert!(status.enabled);
        assert!(autostart.is_enabled());

        let status = autostart.set_enabled(false).unwrap();
        assert!(!status.enabled);
        assert!(!autostart.is_enabled());
    }

    #[test]
    fn reconcile_reenables_after_system_block() {
        let autostart = WinAutoStart {
            backend: Box::new(MemoryRegistry::default()),
        };
        autostart
            .backend
            .write_string(
                RUN_KEY,
                VALUE_NAME,
                &format!("\"{}\"", std::env::current_exe().unwrap().display()),
            )
            .unwrap();
        autostart
            .backend
            .write_string(APPROVED_KEY, VALUE_NAME, "\u{3}")
            .unwrap();
        let status = autostart.reconcile(true).unwrap();
        assert!(status.enabled);
    }

    #[test]
    fn filetime_formats_known_date() {
        let filetime = 133_000_000_000_000_000u64;
        let text = format_filetime(filetime).unwrap();
        assert_eq!(text, "2022-06-18 04:26 UTC");
    }
}
