#![allow(unexpected_cfgs)]

use anyhow::Result;

// ── 共享类型定义（被主 crate 和 platform 各模块使用） ──

/// 二维坐标点
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// 矩形区域
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn min_x(&self) -> f64 {
        self.x
    }

    pub fn min_y(&self) -> f64 {
        self.y
    }

    pub fn max_x(&self) -> f64 {
        self.x + self.width
    }

    pub fn max_y(&self) -> f64 {
        self.y + self.height
    }

    pub fn mid_x(&self) -> f64 {
        self.x + self.width / 2.0
    }

    pub fn mid_y(&self) -> f64 {
        self.y + self.height / 2.0
    }

    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.min_x()
            && point.x <= self.max_x()
            && point.y >= self.min_y()
            && point.y <= self.max_y()
    }
}

/// 显示器信息
#[derive(Debug, Clone)]
pub struct MonitorInfo {
    /// 显示器唯一标识
    pub id: u64,
    /// 显示器完整区域（包含任务栏/Dock）
    pub bounds: Rect,
    /// 可用工作区域（排除任务栏/Dock）
    pub work_area: Rect,
}

// ── 平台抽象 trait ──

/// 鼠标事件
#[derive(Debug, Clone)]
pub struct MouseEvent {
    /// 鼠标点击位置
    pub point: Point,
    /// 鼠标按键编号（2 = 中键）
    pub button: u32,
}

/// 窗口句柄（平台无关的包装）
#[derive(Debug, Clone)]
pub struct WindowHandle {
    #[cfg(target_os = "macos")]
    pub(crate) inner: macos::MacWindowHandle,
    #[cfg(target_os = "windows")]
    pub(crate) inner: windows::WinWindowHandle,
}

/// 鼠标钩子 trait
pub trait MouseHook {
    /// 启动鼠标钩子，接收中键点击回调
    fn start<F>(&mut self, callback: F) -> Result<()>
    where
        F: Fn(MouseEvent) -> bool + Send + 'static;

    /// 停止鼠标钩子
    fn stop(&mut self) -> Result<()>;

    /// 钩子是否处于活跃状态
    fn is_active(&self) -> bool;
}

/// 窗口管理 trait
pub trait WindowManager {
    /// 获取指定位置的窗口
    fn get_window_at(&self, point: Point) -> Option<WindowHandle>;

    /// 获取窗口的 frame（位置 + 尺寸）
    fn get_window_frame(&self, handle: &WindowHandle) -> Option<Rect>;

    /// 设置窗口位置
    fn set_window_position(&self, handle: &WindowHandle, pos: Point) -> Result<()>;

    /// 设置窗口尺寸
    fn set_window_size(&self, handle: &WindowHandle, width: f64, height: f64) -> Result<()>;

    /// 激活窗口（置前 + 获取焦点）
    fn activate_window(&self, handle: &WindowHandle) -> Result<()>;

    /// 窗口是否处于最大化状态
    fn is_maximized(&self, handle: &WindowHandle) -> bool;

    /// 还原窗口
    fn restore_window(&self, handle: &WindowHandle) -> Result<()>;

    /// 最大化窗口
    fn maximize_window(&self, handle: &WindowHandle) -> Result<()>;
}

/// 命中检测 trait
pub trait HitTester {
    /// 判断点击位置是否在窗口标题栏上
    fn is_title_bar_hit(&self, handle: &WindowHandle, point: Point) -> bool;

    /// 判断点击位置是否在交互式标签页上（如浏览器标签、资源管理器标签）
    fn is_interactive_tab(&self, handle: &WindowHandle, point: Point) -> bool;
}

/// 显示器管理 trait
pub trait MonitorManager {
    /// 获取所有显示器信息
    fn get_monitors(&self) -> Vec<MonitorInfo>;

    /// 获取窗口所在的显示器
    fn get_monitor_for_window(&self, handle: &WindowHandle) -> Option<MonitorInfo>;
}

/// 自启动被阻塞的原因
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockReason {
    /// 被系统启动项管理器禁用
    DisabledBySystem { since: Option<String> },
    /// 注册项指向的程序已不存在或与当前程序不一致
    TargetMissing { path: String },
    /// 需要用户在系统设置中批准
    NeedsApproval,
    /// 平台不支持
    Unsupported,
    /// 权限不足
    PermissionDenied,
}

/// 自启动真实状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoStartStatus {
    pub enabled: bool,
    pub requires_user_action: bool,
    pub blocked: Option<BlockReason>,
    pub detail: String,
}

impl AutoStartStatus {
    pub fn enabled(detail: impl Into<String>) -> Self {
        Self {
            enabled: true,
            requires_user_action: false,
            blocked: None,
            detail: detail.into(),
        }
    }

    pub fn disabled(detail: impl Into<String>) -> Self {
        Self {
            enabled: false,
            requires_user_action: false,
            blocked: None,
            detail: detail.into(),
        }
    }

    pub fn blocked(
        blocked: BlockReason,
        requires_user_action: bool,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            enabled: false,
            requires_user_action,
            blocked: Some(blocked),
            detail: detail.into(),
        }
    }
}

/// 开机自启动 trait
pub trait AutoStart {
    /// 读取系统真实状态
    fn status(&self) -> Result<AutoStartStatus>;

    /// 设置自启动状态，返回设置后的真实状态
    fn set_enabled(&self, enabled: bool) -> Result<AutoStartStatus>;

    /// 将系统状态与期望值对齐
    fn reconcile(&self, desired: bool) -> Result<AutoStartStatus> {
        let current = self.status()?;
        if current.enabled == desired && current.blocked.is_none() {
            return Ok(current);
        }
        self.set_enabled(desired)
    }

    /// 是否已启用自启动
    fn is_enabled(&self) -> bool {
        self.status().map(|status| status.enabled).unwrap_or(false)
    }
}

/// 权限检查 trait
pub trait PermissionChecker {
    /// 检查是否有必要的权限（macOS 辅助功能权限 / Windows 管理员权限）
    fn check_permissions(&self) -> bool;

    /// 请求权限（打开系统设置等）
    fn request_permissions(&self) -> Result<()>;
}

// 平台实现模块
#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "windows")]
pub mod windows;

// 平台工厂函数
#[cfg(target_os = "macos")]
pub fn create_platform() -> macos::MacPlatform {
    macos::MacPlatform::new()
}

#[cfg(target_os = "windows")]
pub fn create_platform() -> windows::WinPlatform {
    windows::WinPlatform::new()
}

#[cfg(target_os = "macos")]
pub fn auto_start() -> macos::autostart::MacAutoStart {
    macos::autostart::MacAutoStart::new()
}

#[cfg(target_os = "windows")]
pub fn auto_start() -> windows::autostart::WinAutoStart {
    windows::autostart::WinAutoStart::new()
}
