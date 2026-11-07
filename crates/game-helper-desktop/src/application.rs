//! Native process entry and mode selection.
use gpui_kit::{
    AppContext, Bounds, Context, IntoElement, ParentElement, Render, Styled, Window, WindowBounds,
    WindowOptions, application, assets, div, init, open_window, px, size,
};
use std::path::PathBuf;

struct StartupError(String);
impl Render for StartupError {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().p_5().child(self.0.clone())
    }
}
pub fn run() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.iter().any(|a| a == "--probe") {
        crate::run_probe();
        return;
    }
    let picker = args.iter().any(|a| a == "--picker");
    let explicit = args
        .iter()
        .position(|a| a == "--app-root")
        .map(|i| args.get(i + 1).map(PathBuf::from));
    let exe = std::env::current_exe().ok();
    let root = if let Some(Some(path)) = explicit.as_ref() {
        game_helper_core::storage::AppPaths::new(path)
            .map_err(|e| format!("--app-root 无效：{e:#}"))
    } else if matches!(explicit, Some(None)) {
        Err("--app-root 缺少目录参数".into())
    } else {
        let resolved =
            crate::app_root::ResolvedRoot::resolve(None, exe.as_deref().and_then(|p| p.parent()));
        resolved
            .paths()
            .cloned()
            .ok_or_else(|| resolved.error_message())
    };
    let mut error = None;
    let root = match root {
        Ok(paths) => Some(paths),
        Err(e) => {
            error = Some(e);
            None
        }
    };
    let guard = root
        .as_ref()
        .map(|paths| game_helper_windows::instance::InstanceGuard::acquire(&paths.root, picker));
    let guard = match guard {
        Some(Ok(guard)) => Some(guard),
        Some(Err(e)) => {
            error = Some(format!("{e:#}"));
            None
        }
        None => None,
    };
    if let Err(e) = game_helper_windows::dpi::try_set_per_monitor_v2() {
        error = Some(format!("无法启用 DPI 感知：{e}"));
    }
    // This stack guard lives through the blocking native application loop.
    let _instance_guard = guard;
    application().with_assets(assets::Assets).run(move |cx| {
        init(cx);
        crate::workspace_theme::init(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let options = WindowOptions {
            titlebar: Some(gpui_kit::TitlebarOptions {
                title: Some(
                    if picker {
                        "取点工具"
                    } else {
                        "路线管理"
                    }
                    .into(),
                ),
                ..Default::default()
            }),
            window_min_size: Some(size(px(900.), px(640.))),
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                if picker {
                    size(px(1120.), px(800.))
                } else {
                    size(px(1280.), px(900.))
                },
                cx,
            ))),
            ..Default::default()
        };
        if let Some(error) = error.as_ref() {
            let error = error.clone();
            open_window(options, cx, |_, cx| cx.new(|_| StartupError(error)))
                .expect("无法打开错误窗口");
        } else if let Some(paths) = root.as_ref() {
            let paths = paths.clone();
            if picker {
                open_window(options, cx, |w, cx| {
                    cx.new(|cx| crate::picker::Picker::new(paths, w, cx))
                })
                .expect("无法打开取点窗口");
            } else {
                open_window(options, cx, |w, cx| {
                    cx.new(|cx| crate::manager::Manager::new(paths, w, cx))
                })
                .expect("无法打开管理器");
            }
        }
    });
}
