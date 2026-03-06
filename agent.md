# ScreenHop Agent

## 项目概述

**ScreenHop** 是一个跨平台桌面应用（macOS & Windows），通过鼠标中键点击实现窗口在多显示器之间的快速移动。使用 Rust 构建，追求高性能。

- **仓库**: https://github.com/EcoRoundDev/ScreenHop
- **版本**: 1.0.6
- **许可证**: MIT

## 架构

```
crates/
├── core/          # 共享类型（Point, Rect, MonitorInfo）、配置、显示器几何计算、更新器
│   └── src/
│       ├── lib.rs        # 核心数据结构：Point, Rect, MonitorInfo
│       ├── config.rs     # AppConfig（TOML 序列化，加载/保存）
│       ├── monitor.rs    # 显示器几何计算
│       └── updater.rs    # 自动更新检查（GitHub releases）
├── platform/      # 平台抽象层 + 各系统具体实现
│   └── src/
│       ├── lib.rs        # 核心 trait：MouseHook, WindowManager, HitTester,
│       │                 #   MonitorManager, AutoStart, PermissionChecker
│       ├── macos/
│       │   ├── mod.rs        # MacPlatform 聚合结构体
│       │   ├── hook.rs       # CGEventTap 鼠标事件钩子
│       │   ├── window.rs     # AXUIElement 窗口操作
│       │   ├── hittest.rs    # 标题栏 / 标签页点击检测
│       │   ├── monitor.rs    # CGDisplay 显示器枚举
│       │   └── autostart.rs  # Launch Agent plist 管理
│       └── windows/
│           ├── mod.rs        # WinPlatform 聚合结构体
│           ├── hook.rs       # WH_MOUSE_LL 底层钩子
│           ├── window.rs     # Win32 窗口操作
│           ├── hittest.rs    # 标题栏 / 标签页点击检测
│           ├── monitor.rs    # EnumDisplayMonitors
│           └── autostart.rs  # 任务计划程序自启动
└── app/           # 主入口、系统托盘（tray-icon/muda）、UI（slint）
    ├── build.rs          # Windows 图标/清单嵌入（embed-resource）
    └── src/
        ├── main.rs       # 入口点，权限检查，单实例
        ├── engine.rs     # 鼠标钩子安装 + 窗口移动逻辑
        ├── tray.rs       # 系统托盘菜单
        └── slint_ui.rs   # 设置界面
```

## 构建命令

### macOS (Apple Silicon)
```bash
cargo bundle --release --target aarch64-apple-darwin
codesign --force --deep --sign - target/aarch64-apple-darwin/release/bundle/osx/ScreenHop.app
```

### macOS (Intel)
```bash
cargo bundle --release --target x86_64-apple-darwin
codesign --force --deep --sign - target/x86_64-apple-darwin/release/bundle/osx/ScreenHop.app
```

### macOS (Universal Binary)
```bash
# 构建两种架构后用 lipo 合并
# 完整 CI 流程见 .github/workflows/build.yml
```

### Windows
```bash
cargo build --release --target x86_64-pc-windows-msvc
```

## 代码规范

- **不加注释**，除非明确要求
- 优先使用根 `Cargo.toml` 中定义的 workspace 依赖
- 平台相关代码使用 `#[cfg(target_os = "...")]` 块
- 错误处理：`anyhow::Result<T>` 配合 `.context()` 添加错误信息
- 单实例：TCP 端口 57832 锁
- 日志：`log` crate + `env_logger`，日志写入临时目录

## 关键文件

| 文件 | 用途 |
|------|------|
| `crates/app/src/main.rs` | 入口点，权限检查，单实例 |
| `crates/app/src/engine.rs` | 鼠标钩子安装 + 窗口移动逻辑 |
| `crates/app/src/tray.rs` | 系统托盘菜单 |
| `crates/app/src/slint_ui.rs` | 设置界面 |
| `crates/app/build.rs` | Windows 图标/清单嵌入 |
| `crates/core/src/lib.rs` | 核心类型：`Point`, `Rect`, `MonitorInfo` |
| `crates/core/src/config.rs` | 应用配置（TOML） |
| `crates/core/src/monitor.rs` | 显示器几何计算 |
| `crates/core/src/updater.rs` | 自动更新检查 |
| `crates/platform/src/lib.rs` | 平台 trait：`MouseHook`, `WindowManager`, `HitTester`, `MonitorManager`, `AutoStart`, `PermissionChecker` |
| `crates/platform/src/macos/hook.rs` | macOS 鼠标事件钩子（CGEventTap） |
| `crates/platform/src/macos/window.rs` | macOS 窗口操作（AXUIElement） |
| `crates/platform/src/macos/hittest.rs` | macOS 标题栏 / 标签页点击检测 |
| `crates/platform/src/macos/monitor.rs` | macOS 显示器枚举 |
| `crates/platform/src/macos/autostart.rs` | macOS 自启动（Launch Agent） |
| `crates/platform/src/windows/hook.rs` | Windows 鼠标钩子（WH_MOUSE_LL） |
| `crates/platform/src/windows/window.rs` | Windows 窗口操作 |
| `crates/platform/src/windows/hittest.rs` | Windows 标题栏 / 标签页点击检测 |
| `crates/platform/src/windows/monitor.rs` | Windows 显示器枚举 |
| `crates/platform/src/windows/autostart.rs` | Windows 自启动（任务计划程序） |

## 依赖

### Core (`crates/core`)
- `log` - 日志门面
- `anyhow` - 错误处理
- `serde`, `toml`, `serde_json` - 配置序列化
- `reqwest`, `tokio`, `futures-util` - HTTP / 异步运行时
- `semver` - 版本比较
- `dirs` - 平台目录
- `zip` - 压缩包解压（自动更新）

### App (`crates/app`)
- `tao` - 窗口事件循环
- `muda` - 菜单栏
- `tray-icon` - 系统托盘
- `slint` - UI 框架
- `image`, `imageproc` - 图像处理
- `open` - 浏览器打开 URL
- `env_logger` - 日志输出
- `cocoa`, `objc`（仅 macOS）- Cocoa 绑定
- `windows`（仅 Windows）- Win32 API

### Platform (`crates/platform`)
- `objc2`, `objc2-foundation`, `objc2-app-kit`（macOS）- 现代 Obj-C 绑定
- `core-graphics`, `core-foundation`（macOS）- CoreGraphics API
- `cocoa`, `objc`（macOS）- 旧版 Cocoa 绑定
- `windows`（Windows）- Win32 API（无障碍、GDI、线程、任务计划程序）

## 配置

- **配置路径**：
  - macOS: `~/Library/Application Support/screenhop/config.toml`
  - Windows: `%APPDATA%/screenhop/config.toml`
- **格式**：TOML，通过 `serde` 序列化

## 调试

- **日志文件**：
  - `TEMP/screenhop_run.log` - 运行时日志
  - `TEMP/screenhop_fatal_err.log` - 致命错误日志
- **单实例端口**：127.0.0.1:57832

### macOS 权限
- 需要**辅助功能**权限来注册鼠标钩子
- 检查路径：`系统设置` → `隐私与安全性` → `辅助功能`

## CI/CD

- **工作流**：`.github/workflows/build.yml`
- 构建 macOS（aarch64 + x86_64 + Universal Binary）和 Windows（x86_64）
- 在 GitHub release tag 创建时触发

## 发版流程

1. 更新 `Cargo.toml` 中的版本号（`workspace.package.version`）
2. 在 GitHub 创建 release 并打 tag `vX.X.X`
3. CI 自动构建所有平台产物
4. 上传构建产物到 release（macOS `.dmg` / Windows `.exe`）

## 测试

```bash
cargo test --workspace
```

- 在 macOS 和 Windows 上分别测试
- 验证鼠标钩子在不同显示器配置下正常工作
- 测试窗口在不同分辨率显示器间移动时的缩放行为
- 测试浏览器标签页区域检测

## 忽略文件

以下文件和目录不需要读取或修改，请在工作时忽略：

- `target/` - Rust 编译输出目录
- `**/*.rs.bk` - Rust 备份文件
- `Cargo.lock` - 锁文件，通常很长且不需要 AI 审查
