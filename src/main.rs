mod terminal;
mod sidebar;
mod clone;
mod session;
mod state;

use gpui::*;
use terminal::{ShellCommand, TerminalView};
use terminal::pty_terminal::PtyTerminal;

struct AppState {
    terminal_view: Entity<TerminalView>,
    mode_label: String,
}

fn main() {
    let application = Application::new();

    application.run(move |cx: &mut App| {
        // Detect Claude Code binary
        let (command, label) = if let Some(claude_path) = PtyTerminal::find_claude() {
            let path_str = claude_path.to_string_lossy().to_string();
            eprintln!("Found Claude Code at: {path_str}");
            (
                Some(ShellCommand::new(path_str)),
                "Claude Code".to_string(),
            )
        } else {
            eprintln!("Claude Code not found — falling back to default shell");
            (None, "Shell".to_string())
        };

        cx.open_window(
            WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("CC Multiplex".into()),
                    ..Default::default()
                }),
                window_min_size: Some(size(px(800.0), px(600.0))),
                ..Default::default()
            },
            move |window, cx| {
                let terminal_view = cx.new(|cx| {
                    TerminalView::new(window, cx, command, None)
                });
                cx.new(|_cx| AppState {
                    terminal_view,
                    mode_label: label,
                })
            },
        )
        .unwrap();
    });
}

impl Render for AppState {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .bg(rgb(0x1e1e2e))
            .text_color(rgb(0xcdd6f4))
            .child(
                // Sidebar
                div()
                    .w(px(240.0))
                    .h_full()
                    .bg(rgb(0x181825))
                    .border_r_1()
                    .border_color(rgb(0x313244))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .p(px(12.0))
                            .child("CC Multiplex"),
                    )
                    .child(
                        // Session indicator
                        div()
                            .p(px(12.0))
                            .text_size(px(11.0))
                            .text_color(rgb(0x6c7086))
                            .child(format!("● {}", self.mode_label)),
                    ),
            )
            .child(
                // Main terminal area
                div()
                    .flex_1()
                    .h_full()
                    .child(self.terminal_view.clone()),
            )
    }
}
