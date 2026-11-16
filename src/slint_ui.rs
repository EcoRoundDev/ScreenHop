use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::mpsc::Receiver;

use slint::ComponentHandle;

slint::slint! {
    import { ScrollView } from "std-widgets.slint";
    export global Theme {
        in-out property <bool> dark: false;
        in-out property <string> font-family: "";

        out property <color> bg: dark ? #1C1C1E : #FFFFFF;
        out property <color> surface: dark ? #2C2C2E : #F5F5F7;
        out property <color> surface-hover: dark ? #3A3A3C : #ECECEF;
        out property <color> primary: dark ? #4A84FF : #2F6BFF;
        out property <color> primary-hover: dark ? #6698FF : #1F5AEE;
        out property <color> primary-muted: dark ? #22345C : #C9D8FF;
        out property <color> text: dark ? #F5F5F7 : #1D1D1F;
        out property <color> text-secondary: dark ? #A1A1A6 : #6E6E73;
        out property <color> text-on-accent: #FFFFFF;
        out property <color> border: dark ? #3A3A3C : #D2D2D7;
        out property <color> danger: #E5484D;
        out property <color> danger-hover: #D93A3F;
        out property <color> track: dark ? #3A3A3C : #E3E3E8;
        out property <color> toast-bg: dark ? #3A3A3C : #1D1D1F;

        out property <length> radius-control: 8px;
        out property <length> radius-card: 12px;
        out property <length> radius-dialog: 16px;
        out property <length> gap-1: 4px;
        out property <length> gap-2: 8px;
        out property <length> gap-3: 12px;
        out property <length> gap-4: 16px;
        out property <length> gap-5: 24px;
        out property <length> font-xs: 11px;
        out property <length> font-sm: 12px;
        out property <length> font-md: 13px;
        out property <length> font-lg: 15px;
        out property <length> font-xl: 20px;
        out property <duration> motion: 120ms;
    }

    enum ButtonKind { primary, secondary, ghost, danger }

    component AppButton inherits Rectangle {
        in property <string> text;
        in property <ButtonKind> kind: ButtonKind.secondary;
        in property <bool> enabled: true;
        in property <bool> busy: false;
        callback clicked();

        height: 32px;
        min-width: 76px;
        horizontal-stretch: 0;
        border-radius: Theme.radius-control;
        border-width: kind == ButtonKind.secondary ? 1px : 0px;
        border-color: Theme.border;
        background: !enabled ? Theme.surface
            : kind == ButtonKind.primary ? (ta.has-hover ? Theme.primary-hover : Theme.primary)
            : kind == ButtonKind.danger ? (ta.has-hover ? Theme.danger-hover : Theme.danger)
            : kind == ButtonKind.ghost ? (ta.has-hover ? Theme.surface-hover : transparent)
            : (ta.has-hover ? Theme.surface-hover : Theme.surface);
        animate background { duration: Theme.motion; }

        Text {
            width: parent.width;
            height: parent.height;
            text: root.busy ? "处理中…" : root.text;
            color: !root.enabled ? Theme.text-secondary
                : (root.kind == ButtonKind.primary || root.kind == ButtonKind.danger) ? Theme.text-on-accent
                : Theme.text;
            font-size: Theme.font-md;
            font-weight: 500;
            horizontal-alignment: center;
            vertical-alignment: center;
            overflow: elide;
        }

        ta := TouchArea {
            enabled: root.enabled && !root.busy;
            mouse-cursor: pointer;
            clicked => { root.clicked(); }
        }
    }

    component AppTextField inherits Rectangle {
        in property <string> placeholder;
        in property <bool> password: false;
        in property <bool> enabled: true;
        in-out property <string> text;
        callback edited(string);
        callback accepted();

        height: 32px;
        horizontal-stretch: 1;
        border-radius: Theme.radius-control;
        background: Theme.surface;
        border-width: 1px;
        border-color: ti.has-focus ? Theme.primary : Theme.border;
        clip: true;

        ti := TextInput {
            x: 10px;
            width: parent.width - 20px;
            height: parent.height;
            text <=> root.text;
            enabled: root.enabled;
            color: Theme.text;
            font-size: Theme.font-md;
            vertical-alignment: center;
            input-type: root.password ? InputType.password : InputType.text;
            edited => { root.edited(ti.text); }
            accepted => { root.accepted(); }
        }

        Text {
            x: 10px;
            width: parent.width - 20px;
            height: parent.height;
            text: root.placeholder;
            color: Theme.text-secondary;
            font-size: Theme.font-md;
            vertical-alignment: center;
            overflow: elide;
            visible: root.text == "" && !ti.has-focus;
        }
    }

    component AppField inherits VerticalLayout {
        in property <string> label;
        spacing: 6px;
        alignment: start;
        vertical-stretch: 0;
        height: 54px;

        Text {
            height: 16px;
            text: root.label;
            color: Theme.text-secondary;
            font-size: Theme.font-sm;
            vertical-alignment: center;
        }
    }

    component AppToggle inherits Rectangle {
        in property <bool> enabled: true;
        in-out property <bool> checked: false;
        callback toggled(bool);

        width: 40px;
        height: 22px;
        horizontal-stretch: 0;
        vertical-stretch: 0;
        border-radius: 11px;
        background: !enabled ? Theme.track : (checked ? Theme.primary : Theme.track);
        animate background { duration: Theme.motion; }

        Rectangle {
            width: 18px;
            height: 18px;
            y: 2px;
            x: root.checked ? root.width - 20px : 2px;
            border-radius: 9px;
            background: #FFFFFF;
            drop-shadow-color: #00000033;
            drop-shadow-blur: 2px;
            animate x { duration: Theme.motion; }
        }

        ta := TouchArea {
            enabled: root.enabled;
            mouse-cursor: pointer;
            clicked => { if (root.enabled) { root.toggled(!root.checked); } }
        }
    }

    component AppSegmented inherits Rectangle {
        in property <[string]> options: [];
        in property <int> current: 0;
        callback changed(int);

        height: 32px;
        border-radius: Theme.radius-control;
        background: Theme.surface;
        border-width: 1px;
        border-color: Theme.border;

        HorizontalLayout {
            padding: 2px;
            spacing: 2px;
            for option[idx] in options : Rectangle {
                border-radius: 6px;
                background: idx == root.current ? Theme.bg : transparent;
                Text {
                    width: parent.width;
                    height: parent.height;
                    text: option;
                    color: idx == root.current ? Theme.text : Theme.text-secondary;
                    font-size: Theme.font-sm;
                    horizontal-alignment: center;
                    vertical-alignment: center;
                }
                TouchArea {
                    mouse-cursor: pointer;
                    clicked => { root.changed(idx); }
                }
            }
        }
    }

    component AppProgressBar inherits Rectangle {
        in property <float> value: 0;

        height: 6px;
        horizontal-stretch: 1;
        border-radius: 3px;
        background: Theme.track;
        clip: true;

        Rectangle {
            x: 0;
            y: 0;
            height: parent.height;
            width: max(0px, min(parent.width, parent.width * root.value));
            border-radius: 3px;
            background: Theme.primary;
            animate width { duration: 200ms; }
        }
    }

    component AppSlider inherits Rectangle {
        in property <float> minimum: 20;
        in property <float> maximum: 80;
        in-out property <float> value: 40;
        callback changed(float);

        height: 24px;
        horizontal-stretch: 1;

        track := Rectangle {
            x: 8px;
            y: (parent.height - 4px) / 2;
            width: parent.width - 16px;
            height: 4px;
            border-radius: 2px;
            background: Theme.track;
        }

        Rectangle {
            x: track.x;
            y: track.y;
            height: track.height;
            width: max(0px, track.width * (root.value - root.minimum) / (root.maximum - root.minimum));
            border-radius: 2px;
            background: Theme.primary;
        }

        Rectangle {
            width: 16px;
            height: 16px;
            border-radius: 8px;
            y: (parent.height - self.height) / 2;
            x: min(parent.width - self.width, max(0px, track.x + track.width * (root.value - root.minimum) / (root.maximum - root.minimum) - self.width / 2));
            background: #FFFFFF;
            border-width: 1px;
            border-color: Theme.border;
            drop-shadow-color: #00000040;
            drop-shadow-blur: 3px;
        }

        ta := TouchArea {
            mouse-cursor: pointer;
            clicked => { root.commit(self.mouse-x); }
            moved => { if (self.pressed) { root.commit(self.mouse-x); } }
        }

        function commit(px: length) {
            root.value = max(root.minimum, min(root.maximum, Math.round(root.minimum + (px - track.x) / track.width * (root.maximum - root.minimum))));
            root.changed(root.value);
        }
    }

    component AppCard inherits Rectangle {
        border-radius: Theme.radius-card;
        background: Theme.bg;
        border-width: 1px;
        border-color: Theme.border;
        drop-shadow-color: #0000000F;
        drop-shadow-blur: 6px;
        drop-shadow-offset-y: 1px;

        VerticalLayout {
            padding: 16px;
            spacing: 12px;
            @children
        }
    }

    component NavItem inherits Rectangle {
        in property <string> text;
        in property <bool> active: false;
        callback clicked();

        height: 36px;
        border-radius: Theme.radius-control;
        background: active ? Theme.bg : (ta.has-hover ? Theme.surface-hover : transparent);
        animate background { duration: Theme.motion; }

        ta := TouchArea {
            mouse-cursor: pointer;
            clicked => { root.clicked(); }
        }

        Text {
            x: 12px;
            width: parent.width - 24px;
            height: parent.height;
            text: root.text;
            color: root.active ? Theme.primary : Theme.text;
            font-size: Theme.font-md;
            font-weight: root.active ? 600 : 400;
            vertical-alignment: center;
        }
    }

    component WindowGlyph inherits Rectangle {
        in property <bool> is-close: false;
        in property <color> glyph-color: #000000;

        if root.is-close : Path {
            width: 10px;
            height: 10px;
            x: (parent.width - self.width) / 2;
            y: (parent.height - self.height) / 2;
            stroke: root.glyph-color;
            stroke-width: 1.2px;
            commands: "M 1 1 L 9 9 M 9 1 L 1 9";
        }

        if !root.is-close : Rectangle {
            width: 10px;
            height: 1.2px;
            x: (parent.width - self.width) / 2;
            y: (parent.height - self.height) / 2;
            background: root.glyph-color;
        }
    }

    component WindowButton inherits Rectangle {
        in property <bool> is-close: false;
        in property <bool> danger: false;
        callback clicked();

        width: 44px;
        vertical-stretch: 1;
        background: ta.has-hover ? (root.danger ? Theme.danger : Theme.surface-hover) : transparent;
        animate background { duration: Theme.motion; }

        WindowGlyph {
            width: parent.width;
            height: parent.height;
            is-close: root.is-close;
            glyph-color: (ta.has-hover && root.danger) ? #FFFFFF : Theme.text-secondary;
        }

        ta := TouchArea {
            mouse-cursor: pointer;
            clicked => { root.clicked(); }
        }
    }

    component AppTitleBar inherits Rectangle {
        in property <string> title;
        in property <bool> show-minimize: true;
        callback close();
        callback minimize();
        callback drag-request();

        height: 40px;
        background: Theme.surface;

        drag-area := TouchArea {
            width: root.show-minimize ? parent.width - 88px : parent.width - 44px;
            height: parent.height;
            pointer-event(e) => {
                if (e.button == PointerEventButton.left && e.kind == PointerEventKind.down) {
                    root.drag-request();
                }
            }
        }

        Text {
            x: 16px;
            width: parent.width - 140px;
            height: parent.height;
            text: root.title;
            color: Theme.text-secondary;
            font-size: Theme.font-sm;
            vertical-alignment: center;
            overflow: elide;
        }

        WindowButton {
            x: parent.width - 88px;
            width: 44px;
            height: parent.height;
            visible: root.show-minimize;
            clicked => { root.minimize(); }
        }

        WindowButton {
            x: parent.width - 44px;
            width: 44px;
            height: parent.height;
            is-close: true;
            danger: true;
            clicked => { root.close(); }
        }
    }

    component AppToast inherits Rectangle {
        in property <string> text;

        height: 36px;
        border-radius: 18px;
        background: Theme.toast-bg;
        drop-shadow-color: #00000033;
        drop-shadow-blur: 10px;

        Text {
            x: 16px;
            width: parent.width - 32px;
            height: parent.height;
            text: root.text;
            color: #FFFFFF;
            font-size: Theme.font-sm;
            horizontal-alignment: center;
            vertical-alignment: center;
            overflow: elide;
        }
    }

    export component AlertDialog inherits Window {
        title: "ScreenHop";
        width: 440px;
        height: body-layout.preferred-height + 148px;
        no-frame: true;
        background: Theme.bg;
        default-font-family: Theme.font-family;

        in property <string> heading: "提示";
        in property <string> body: "";
        in property <bool> danger: false;
        in property <bool> ask: false;
        in property <string> ok-text: "知道了";
        callback ok();
        callback cancel();

        VerticalLayout {
            padding: 24px;
            spacing: 16px;

            Text {
                text: root.heading;
                color: root.danger ? Theme.danger : Theme.text;
                font-size: Theme.font-lg;
                font-weight: 600;
                vertical-alignment: center;
            }

            body-layout := Text {
                text: root.body;
                color: Theme.text-secondary;
                font-size: Theme.font-md;
                wrap: word-wrap;
                overflow: elide;
                vertical-alignment: top;
            }

            HorizontalLayout {
                alignment: end;
                spacing: 8px;

                AppButton {
                    text: "取消";
                    kind: ButtonKind.ghost;
                    visible: root.ask;
                    clicked => { root.cancel(); }
                }

                AppButton {
                    text: root.ok-text;
                    kind: root.danger ? ButtonKind.danger : ButtonKind.primary;
                    clicked => { root.ok(); }
                }
            }
        }
    }

    export component UpdateProgressDialog inherits Window {
        title: "更新 ScreenHop";
        width: 460px;
        height: 216px;
        no-frame: true;
        background: Theme.bg;
        default-font-family: Theme.font-family;

        in-out property <string> status_text: "正在准备更新…";
        in-out property <float> progress: 0;
        in-out property <bool> can_cancel: true;
        callback cancel();

        VerticalLayout {

            AppTitleBar {
                title: "更新 ScreenHop";
                show-minimize: false;
                close => { root.cancel(); }
                minimize => {}
                drag-request => {}
            }

            VerticalLayout {
                padding: 24px;
                spacing: 16px;

                Text {
                    text: root.status_text;
                    color: Theme.text;
                    font-size: Theme.font-md;
                    wrap: word-wrap;
                    vertical-alignment: center;
                }

                AppProgressBar { value: root.progress; }

                HorizontalLayout {
                    alignment: end;
                    AppButton {
                        text: "取消";
                        kind: ButtonKind.secondary;
                        enabled: root.can_cancel;
                        clicked => { root.cancel(); }
                    }
                }
            }
        }
    }

    export component ProxyAuthDialog inherits Window {
        title: "代理设置";
        width: 480px;
        height: 456px;
        no-frame: true;
        background: Theme.bg;
        default-font-family: Theme.font-family;

        in-out property <string> address: "127.0.0.1";
        in-out property <string> port: "2888";
        in-out property <string> protocol: "SOCKS5";
        in-out property <bool> auth_enabled: false;
        in-out property <string> username: "";
        in-out property <string> password: "";
        callback apply(string, string, string, bool, string, string);
        callback cancel();

        VerticalLayout {

            AppTitleBar {
                title: "代理设置";
                show-minimize: false;
                close => { root.cancel(); }
                minimize => {}
                drag-request => {}
            }

            VerticalLayout {
                padding: 24px;
                spacing: 12px;

                Text {
                    text: "协议";
                    color: Theme.text-secondary;
                    font-size: Theme.font-sm;
                    vertical-alignment: center;
                }

                AppSegmented {
                    options: ["SOCKS5", "SOCKS4", "HTTP", "HTTPS"];
                    current: root.protocol == "SOCKS4" ? 1 : (root.protocol == "HTTP" ? 2 : (root.protocol == "HTTPS" ? 3 : 0));
                    changed(idx) => {
                        root.protocol = idx == 1 ? "SOCKS4" : (idx == 2 ? "HTTP" : (idx == 3 ? "HTTPS" : "SOCKS5"));
                    }
                }

                HorizontalLayout {
                    spacing: 12px;

                    AppField {
                        label: "地址";
                        horizontal-stretch: 3;
                        AppTextField {
                            placeholder: "127.0.0.1";
                            text <=> root.address;
                        }
                    }

                    AppField {
                        label: "端口";
                        horizontal-stretch: 1;
                        AppTextField {
                            placeholder: "1080";
                            text <=> root.port;
                        }
                    }
                }

                HorizontalLayout {
                    spacing: 8px;
                    Text {
                        text: "使用用户名与密码认证";
                        color: Theme.text;
                        font-size: Theme.font-md;
                        vertical-alignment: center;
                        horizontal-stretch: 1;
                    }
                    AppToggle {
                        checked: root.auth_enabled;
                        toggled(value) => { root.auth_enabled = value; }
                    }
                }

                VerticalLayout {
                    spacing: 8px;
                    visible: root.auth_enabled;
                    AppField {
                        label: "用户名";
                        AppTextField {
                            placeholder: "用户名";
                            text <=> root.username;
                        }
                    }
                    AppField {
                        label: "密码";
                        AppTextField {
                            placeholder: "密码";
                            password: true;
                            text <=> root.password;
                        }
                    }
                }

                Rectangle { vertical-stretch: 1; }

                HorizontalLayout {
                    alignment: end;
                    spacing: 8px;

                    AppButton {
                        text: "取消";
                        kind: ButtonKind.ghost;
                        clicked => { root.cancel(); }
                    }

                    AppButton {
                        text: "保存";
                        kind: ButtonKind.primary;
                        clicked => {
                            root.apply(root.address, root.port, root.protocol, root.auth_enabled, root.username, root.password);
                        }
                    }
                }
            }
        }
    }

    export component MainWindow inherits Window {
        title: root.app-name + " 设置";
        width: 880px;
        height: 620px;
        no-frame: true;
        background: Theme.bg;
        default-font-family: Theme.font-family;

        in-out property <int> page: 0;

        in-out property <bool> hook-enabled: true;
        in-out property <float> title-height: 40;
        in-out property <string> title-height-text: "40 px";
        in-out property <bool> autostart-enabled: false;
        in-out property <string> autostart-detail: "";
        in-out property <bool> autostart-blocked: false;
        in-out property <bool> hook-restart-hint: false;
        in property <bool> macos-permissions: false;
        in property <bool> dev-build: false;
        in property <string> app-name: "ScreenHop";
        in property <int> permission-state: 0;
        in property <string> permission-status: "正在检测权限";
        in property <string> permission-detail: "";
        in property <string> application-path: "";

        in-out property <bool> proxy-enabled: false;
        in-out property <string> proxy-protocol: "SOCKS5";
        in-out property <string> proxy-address: "127.0.0.1";
        in-out property <string> proxy-port: "2888";
        in-out property <bool> proxy-auth: false;
        in-out property <string> proxy-username: "";
        in-out property <string> proxy-password: "";

        in-out property <bool> auto-check: true;
        in-out property <string> version-text: "";
        in-out property <string> last-check-text: "";
        in-out property <bool> check-busy: false;

        in-out property <string> log-path: "";

        in-out property <bool> toast-visible: false;
        in-out property <string> toast-text: "";

        callback drag-request();
        callback close-requested();
        callback minimize-requested();
        callback hook-toggled(bool);
        callback authorize();
        callback recheck-permission();
        callback reveal-application();
        callback restart-application();
        callback permission-help();
        callback title-height-changed(float);
        callback autostart-toggled(bool);
        callback fix-autostart();
        callback proxy-enabled-toggled(bool);
        callback save-proxy();
        callback auto-check-toggled(bool);
        callback check-now();
        callback open-repo();
        callback open-logs();

        VerticalLayout {

            AppTitleBar {
                title: root.app-name + " 设置";
                close => { root.close-requested(); }
                minimize => { root.minimize-requested(); }
                drag-request => { root.drag-request(); }
            }

            HorizontalLayout {
                Rectangle {
                    width: 200px;
                    background: Theme.surface;

                    VerticalLayout {
                        padding: 12px;
                        spacing: 4px;

                        NavItem { text: "常规"; active: root.page == 0; clicked => { root.page = 0; } }
                        NavItem { text: "代理"; active: root.page == 1; clicked => { root.page = 1; } }
                        NavItem { text: "更新"; active: root.page == 2; clicked => { root.page = 2; } }
                        NavItem { text: "关于"; active: root.page == 3; clicked => { root.page = 3; } }

                        Rectangle { vertical-stretch: 1; }

                        Text {
                            text: root.app-name + " " + root.version-text;
                            color: Theme.text-secondary;
                            font-size: Theme.font-xs;
                            vertical-alignment: center;
                        }
                    }
                }

                Rectangle {
                    background: Theme.bg;
                    horizontal-stretch: 1;

                    if root.page == 0 : ScrollView {
                        VerticalLayout {
                        alignment: start;
                        padding: 24px;
                        spacing: 16px;

                        Text {
                            text: "常规";
                            color: Theme.text;
                            font-size: Theme.font-xl;
                            font-weight: 600;
                            vertical-alignment: center;
                        }

                        AppCard {
                            HorizontalLayout {
                                spacing: 12px;
                                Text {
                                    text: "鼠标中键点击移动窗口";
                                    color: Theme.text;
                                    font-size: Theme.font-md;
                                    vertical-alignment: center;
                                    horizontal-stretch: 1;
                                }
                                AppToggle {
                                    checked: root.hook-enabled;
                                    toggled(value) => { root.hook-enabled = value; root.hook-toggled(value); }
                                }
                            }
                            if root.macos-permissions : VerticalLayout {
                                spacing: 8px;
                                Text {
                                    text: root.permission-status;
                                    color: root.permission-state == 0 || root.permission-state == 4 ? Theme.danger : Theme.text;
                                    font-size: Theme.font-md;
                                    font-weight: 600;
                                }
                                Text {
                                    text: root.permission-detail;
                                    color: Theme.text-secondary;
                                    font-size: Theme.font-sm;
                                    wrap: word-wrap;
                                }
                                HorizontalLayout {
                                    spacing: 8px;
                                    alignment: start;
                                    if root.permission-state == 0 : AppButton {
                                        text: "前往授权";
                                        kind: ButtonKind.primary;
                                        clicked => { root.authorize(); }
                                    }
                                    AppButton {
                                        text: root.permission-state == 4 ? "重试" : "重新检测";
                                        clicked => { root.recheck-permission(); }
                                    }
                                    AppButton {
                                        text: "权限帮助";
                                        kind: ButtonKind.ghost;
                                        clicked => { root.permission-help(); }
                                    }
                                    if root.permission-state == 0 || root.permission-state == 4 : AppButton {
                                        text: "重新启动";
                                        clicked => { root.restart-application(); }
                                    }
                                }
                                HorizontalLayout {
                                    spacing: 8px;
                                    Text {
                                        text: root.application-path;
                                        color: Theme.text-secondary;
                                        font-size: Theme.font-xs;
                                        overflow: elide;
                                        vertical-alignment: center;
                                        horizontal-stretch: 1;
                                    }
                                    AppButton {
                                        text: "在 Finder 中显示";
                                        kind: ButtonKind.ghost;
                                        clicked => { root.reveal-application(); }
                                    }
                                }
                            }
                            Text {
                                text: "关闭后鼠标钩子不再安装，重启应用生效。";
                                color: Theme.text-secondary;
                                font-size: Theme.font-sm;
                                wrap: word-wrap;
                                visible: root.hook-restart-hint;
                            }
                        }

                        AppCard {
                            HorizontalLayout {
                                spacing: 12px;
                                Text {
                                    text: "标题栏高度";
                                    color: Theme.text;
                                    font-size: Theme.font-md;
                                    vertical-alignment: center;
                                }
                                AppSlider {
                                    minimum: 20;
                                    maximum: 80;
                                    value: root.title-height;
                                    changed(value) => { root.title-height = value; root.title-height-changed(value); }
                                }
                                Text {
                                    text: root.title-height-text;
                                    color: Theme.text-secondary;
                                    font-size: Theme.font-sm;
                                    min-width: 48px;
                                    horizontal-alignment: right;
                                    vertical-alignment: center;
                                }
                            }
                            Text {
                                text: "用于判断窗口标题栏区域，影响中键点击时是否触发移动。";
                                color: Theme.text-secondary;
                                font-size: Theme.font-sm;
                                wrap: word-wrap;
                            }
                        }

                        AppCard {
                            HorizontalLayout {
                                spacing: 12px;
                                Text {
                                    text: "开机自动启动";
                                    color: Theme.text;
                                    font-size: Theme.font-md;
                                    vertical-alignment: center;
                                    horizontal-stretch: 1;
                                }
                                AppButton {
                                    text: "修复";
                                    kind: ButtonKind.secondary;
                                    visible: root.autostart-blocked;
                                    clicked => { root.fix-autostart(); }
                                }
                                AppToggle {
                                    enabled: !root.dev-build;
                                    checked: root.autostart-enabled;
                                    toggled(value) => { root.autostart-enabled = value; root.autostart-toggled(value); }
                                }
                            }
                            Text {
                                text: root.autostart-detail;
                                color: root.autostart-blocked ? Theme.danger : Theme.text-secondary;
                                font-size: Theme.font-sm;
                                wrap: word-wrap;
                            }
                        }

                        Rectangle { vertical-stretch: 1; }
                    }

                    }

                    if root.page == 1 : VerticalLayout {
                        padding: 24px;
                        spacing: 16px;

                        Text {
                            text: "代理";
                            color: Theme.text;
                            font-size: Theme.font-xl;
                            font-weight: 600;
                            vertical-alignment: center;
                        }

                        AppCard {
                            vertical-stretch: 0;

                            Rectangle {
                                height: 32px;

                                l1 := HorizontalLayout {
                                    spacing: 12px;
                                    Text {
                                        text: "通过代理检查更新";
                                        color: Theme.text;
                                        font-size: Theme.font-md;
                                        vertical-alignment: center;
                                        horizontal-stretch: 1;
                                    }
                                    AppToggle {
                                        y: (parent.height - self.height) / 2;
                                        checked: root.proxy-enabled;
                                        toggled(value) => { root.proxy-enabled = value; root.proxy-enabled-toggled(value); }
                                    }
                                }
                            }

                            AppField {
                                label: "协议";
                                height: 54px;

                                AppSegmented {
                                    options: ["SOCKS5", "SOCKS4", "HTTP", "HTTPS"];
                                    current: root.proxy-protocol == "SOCKS4" ? 1 : (root.proxy-protocol == "HTTP" ? 2 : (root.proxy-protocol == "HTTPS" ? 3 : 0));
                                    changed(idx) => {
                                        root.proxy-protocol = idx == 1 ? "SOCKS4" : (idx == 2 ? "HTTP" : (idx == 3 ? "HTTPS" : "SOCKS5"));
                                    }
                                }
                            }

                            HorizontalLayout {
                                height: 54px;
                                spacing: 12px;
                                AppField {
                                    label: "地址";
                                    horizontal-stretch: 3;
                                    AppTextField { placeholder: "127.0.0.1"; text <=> root.proxy-address; }
                                }
                                AppField {
                                    label: "端口";
                                    horizontal-stretch: 1;
                                    AppTextField { placeholder: "1080"; text <=> root.proxy-port; }
                                }
                            }

                            Rectangle {
                                height: 32px;

                                HorizontalLayout {
                                    spacing: 8px;
                                    Text {
                                        text: "使用用户名与密码认证";
                                        color: Theme.text;
                                        font-size: Theme.font-md;
                                        vertical-alignment: center;
                                        horizontal-stretch: 1;
                                    }
                                    AppToggle {
                                        y: (parent.height - self.height) / 2;
                                        checked: root.proxy-auth;
                                        toggled(value) => { root.proxy-auth = value; }
                                    }
                                }
                            }

                            if root.proxy-auth : HorizontalLayout {
                                height: 32px;
                                spacing: 12px;
                                AppTextField { placeholder: "用户名"; text <=> root.proxy-username; }
                                AppTextField { placeholder: "密码"; password: true; text <=> root.proxy-password; }
                            }

                            Rectangle {
                                height: 32px;
                                vertical-stretch: 0;

                                HorizontalLayout {
                                    alignment: end;
                                    spacing: 8px;
                                    AppButton {
                                        text: "保存";
                                        kind: ButtonKind.primary;
                                        clicked => { root.save-proxy(); }
                                    }
                                }
                            }
                        }

                        Rectangle { vertical-stretch: 1; }
                    }

                    if root.page == 2 : VerticalLayout {
                        padding: 24px;
                        spacing: 16px;

                        Text {
                            text: "更新";
                            color: Theme.text;
                            font-size: Theme.font-xl;
                            font-weight: 600;
                            vertical-alignment: center;
                        }

                        AppCard {
                            HorizontalLayout {
                                spacing: 12px;
                                Text {
                                    text: "当前版本";
                                    color: Theme.text;
                                    font-size: Theme.font-md;
                                    vertical-alignment: center;
                                    horizontal-stretch: 1;
                                }
                                Text {
                                    text: root.version-text;
                                    color: Theme.text-secondary;
                                    font-size: Theme.font-md;
                                    vertical-alignment: center;
                                }
                            }

                            HorizontalLayout {
                                spacing: 12px;
                                Text {
                                    text: "启动时自动检查更新";
                                    color: Theme.text;
                                    font-size: Theme.font-md;
                                    vertical-alignment: center;
                                    horizontal-stretch: 1;
                                }
                                AppToggle {
                                    enabled: !root.dev-build;
                                    checked: root.auto-check;
                                    toggled(value) => { root.auto-check = value; root.auto-check-toggled(value); }
                                }
                            }

                            Text {
                                text: root.last-check-text;
                                color: Theme.text-secondary;
                                font-size: Theme.font-sm;
                                wrap: word-wrap;
                            }

                            HorizontalLayout {
                                alignment: end;
                                AppButton {
                                    text: root.dev-build ? "开发版不检查更新" : "立即检查";
                                    enabled: !root.dev-build;
                                    kind: ButtonKind.primary;
                                    busy: root.check-busy;
                                    clicked => { root.check-now(); }
                                }
                            }
                        }

                        Rectangle { vertical-stretch: 1; }
                    }

                    if root.page == 3 : VerticalLayout {
                        padding: 24px;
                        spacing: 16px;

                        Text {
                            text: "关于";
                            color: Theme.text;
                            font-size: Theme.font-xl;
                            font-weight: 600;
                            vertical-alignment: center;
                        }

                        AppCard {
                            HorizontalLayout {
                                spacing: 12px;
                                Text {
                                    text: "ScreenHop";
                                    color: Theme.text;
                                    font-size: Theme.font-md;
                                    vertical-alignment: center;
                                    horizontal-stretch: 1;
                                }
                                Text {
                                    text: "v" + root.version-text;
                                    color: Theme.text-secondary;
                                    font-size: Theme.font-md;
                                    vertical-alignment: center;
                                }
                            }
                            Text {
                                text: "用鼠标中键把窗口在多个显示器之间快速移动。";
                                color: Theme.text-secondary;
                                font-size: Theme.font-sm;
                                wrap: word-wrap;
                            }
                            HorizontalLayout {
                                alignment: end;
                                AppButton {
                                    text: "打开仓库";
                                    kind: ButtonKind.secondary;
                                    clicked => { root.open-repo(); }
                                }
                            }
                        }

                        AppCard {
                            Text {
                                text: "日志文件";
                                color: Theme.text;
                                font-size: Theme.font-md;
                                vertical-alignment: center;
                            }
                            Text {
                                text: root.log-path;
                                color: Theme.text-secondary;
                                font-size: Theme.font-sm;
                                wrap: word-wrap;
                            }
                            HorizontalLayout {
                                alignment: end;
                                AppButton {
                                    text: "打开日志目录";
                                    kind: ButtonKind.secondary;
                                    width: 96px;
                                    clicked => { root.open-logs(); }
                                }
                            }
                        }

                        Rectangle { vertical-stretch: 1; }
                    }
                }
            }
        }

        AppToast {
            x: (parent.width - 320px) / 2;
            y: parent.height - 56px;
            width: 320px;
            text: root.toast-text;
            visible: root.toast-visible;
        }

        Timer {
            interval: 2600ms;
            running: root.toast-visible;
            triggered => { root.toast-visible = false; }
        }
    }
}

static SYSTEM_DARK: AtomicU8 = AtomicU8::new(SYSTEM_DARK_UNKNOWN);
static SYSTEM_DARK_INFLIGHT: AtomicBool = AtomicBool::new(false);
static THEME_REFRESHED: AtomicBool = AtomicBool::new(false);

const SYSTEM_DARK_UNKNOWN: u8 = 2;

pub fn cached_system_dark() -> Option<bool> {
    match SYSTEM_DARK.load(Ordering::Relaxed) {
        0 => Some(false),
        1 => Some(true),
        _ => None,
    }
}

fn query_system_dark() -> Option<bool> {
    #[cfg(target_os = "windows")]
    {
        let output = std::process::Command::new("reg")
            .args([
                "query",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
                "/v",
                "AppsUseLightTheme",
            ])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&output.stdout);
        let line = text.lines().find(|l| l.contains("AppsUseLightTheme"))?;
        let value = line.split_whitespace().last()?;
        let normalized = value.trim().trim_start_matches("0x");
        Some(normalized == "0")
    }
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&output.stdout).contains("Dark"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        None
    }
}

pub fn refresh_system_dark<F>(on_done: F)
where
    F: FnOnce(Option<bool>) + Send + 'static,
{
    if SYSTEM_DARK_INFLIGHT.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let started = std::time::Instant::now();
        let value = query_system_dark();
        SYSTEM_DARK_INFLIGHT.store(false, Ordering::SeqCst);
        if let Some(dark) = value {
            SYSTEM_DARK.store(u8::from(dark), Ordering::Relaxed);
            THEME_REFRESHED.store(true, Ordering::SeqCst);
        }
        log::info!(
            "系统主题查询完成: {:?}，耗时 {} ms",
            value,
            started.elapsed().as_millis()
        );
        on_done(value);
    });
}

pub fn take_theme_refresh() -> bool {
    THEME_REFRESHED.swap(false, Ordering::SeqCst)
}

pub fn effective_dark(theme: screenhop::config::Theme) -> bool {
    match theme {
        screenhop::config::Theme::Light => false,
        screenhop::config::Theme::Dark => true,
        screenhop::config::Theme::Auto => cached_system_dark().unwrap_or(false),
    }
}

pub fn platform_font_family() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "Microsoft YaHei UI"
    }
    #[cfg(target_os = "macos")]
    {
        "Hiragino Sans GB"
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        ""
    }
}

fn set_theme(theme: Theme, dark: bool) {
    theme.set_dark(dark);
}

fn initialize_window_style(theme: Theme, dark: bool) {
    theme.set_font_family(slint::SharedString::from(platform_font_family()));
    set_theme(theme, dark);
}

pub fn apply_theme_main(handle: &MainWindow, dark: bool) {
    initialize_window_style(handle.global::<Theme>(), dark);
}

pub fn apply_theme_alert(handle: &AlertDialog, dark: bool) {
    initialize_window_style(handle.global::<Theme>(), dark);
}

pub fn apply_theme_progress(handle: &UpdateProgressDialog, dark: bool) {
    initialize_window_style(handle.global::<Theme>(), dark);
}

pub fn apply_theme_proxy(handle: &ProxyAuthDialog, dark: bool) {
    initialize_window_style(handle.global::<Theme>(), dark);
}

pub fn activate_window(window: &slint::Window) {
    #[cfg(target_os = "macos")]
    #[allow(deprecated)]
    unsafe {
        use cocoa::appkit::NSApp;
        use objc::*;
        let app = NSApp();
        let _: () = msg_send![app, activateIgnoringOtherApps: true];
    }

    use slint::winit_030::WinitWindowAccessor;
    let handled =
        window.with_winit_window(|winit_window: &slint::winit_030::winit::window::Window| {
            if winit_window.is_minimized().unwrap_or(false) {
                winit_window.set_minimized(false);
            }
            winit_window.focus_window();
        });
    if handled.is_none() {
        log::debug!("窗口尚未由 winit 后端创建，跳过系统激活");
    }
}

pub fn show_alert(title: &str, body: &str) {
    let (title, body) = (title.to_string(), body.to_string());
    let _ = slint::invoke_from_event_loop(move || {
        let Ok(dialog) = AlertDialog::new() else {
            return;
        };
        apply_theme_alert(&dialog, effective_dark(screenhop::config::Theme::Auto));
        dialog.set_heading(title.into());
        dialog.set_body(body.into());
        let weak = dialog.as_weak();
        dialog.on_ok(move || {
            if let Some(dialog) = weak.upgrade() {
                let _ = dialog.hide();
            }
        });
        let weak = dialog.as_weak();
        dialog.on_cancel(move || {
            if let Some(dialog) = weak.upgrade() {
                let _ = dialog.hide();
            }
        });
        let _ = dialog.show();
        activate_window(&dialog.window());
    });
}

pub fn show_error(title: &str, body: &str) {
    let (title, body) = (title.to_string(), body.to_string());
    let _ = slint::invoke_from_event_loop(move || {
        let Ok(dialog) = AlertDialog::new() else {
            return;
        };
        apply_theme_alert(&dialog, effective_dark(screenhop::config::Theme::Auto));
        dialog.set_heading(title.into());
        dialog.set_body(body.into());
        dialog.set_danger(true);
        let weak = dialog.as_weak();
        dialog.on_ok(move || {
            if let Some(dialog) = weak.upgrade() {
                let _ = dialog.hide();
            }
        });
        let weak = dialog.as_weak();
        dialog.on_cancel(move || {
            if let Some(dialog) = weak.upgrade() {
                let _ = dialog.hide();
            }
        });
        let _ = dialog.show();
        activate_window(&dialog.window());
    });
}

pub fn confirm(title: &str, body: &str, ask: bool, ok_text: &str) -> Receiver<bool> {
    let (tx, rx) = std::sync::mpsc::channel();
    let (title, body, ok_text) = (title.to_string(), body.to_string(), ok_text.to_string());
    let _ = slint::invoke_from_event_loop(move || {
        let Ok(dialog) = AlertDialog::new() else {
            let _ = tx.send(false);
            return;
        };
        apply_theme_alert(&dialog, effective_dark(screenhop::config::Theme::Auto));
        dialog.set_heading(title.into());
        dialog.set_body(body.into());
        dialog.set_ask(ask);
        dialog.set_ok_text(ok_text.into());
        let weak = dialog.as_weak();
        let tx_ok = tx.clone();
        dialog.on_ok(move || {
            let _ = tx_ok.send(true);
            if let Some(dialog) = weak.upgrade() {
                let _ = dialog.hide();
            }
        });
        let weak = dialog.as_weak();
        let tx_cancel = tx.clone();
        dialog.on_cancel(move || {
            let _ = tx_cancel.send(false);
            if let Some(dialog) = weak.upgrade() {
                let _ = dialog.hide();
            }
        });
        let _ = dialog.show();
        activate_window(&dialog.window());
    });
    rx
}

pub fn bind_main_window(window: &MainWindow) {
    let weak = window.as_weak();
    window.on_drag_request(move || {
        if let Some(window) = weak.upgrade() {
            request_system_drag(&window.window());
        }
    });
}

pub fn request_system_drag(window: &slint::Window) {
    use slint::winit_030::WinitWindowAccessor;
    let started =
        window.with_winit_window(|winit_window: &slint::winit_030::winit::window::Window| {
            winit_window.drag_window()
        });
    match started {
        Some(Ok(())) => {}
        Some(Err(err)) => log::warn!("系统拖动启动失败: {err}"),
        None => log::warn!("系统拖动不可用：窗口尚未由 winit 后端创建"),
    }
}

pub fn format_unix_ts(ts: i64) -> String {
    let days = ts.div_euclid(86_400);
    let seconds = ts.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}",
        year,
        month,
        day,
        seconds / 3600,
        (seconds % 3600) / 60
    )
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}
