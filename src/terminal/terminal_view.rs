use super::grid_element::TerminalGridElement;
use super::pty_terminal::{PtyTerminal, ShellCommand, TermSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use gpui::*;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const FONT_FAMILY: &str = "JetBrains Mono";
const FONT_SIZE: f32 = 13.0;
const MIN_COLS: u16 = 20;
const MIN_ROWS: u16 = 4;

/// Events emitted by the terminal view to the parent
#[derive(Debug, Clone)]
pub enum TerminalEvent {
    NewSession,
    CloseSession,
    SwitchSession(usize), // 0-indexed
}

impl EventEmitter<TerminalEvent> for TerminalView {}

/// GPUI View wrapping a PTY-backed terminal
pub struct TerminalView {
    terminal: Option<PtyTerminal>,
    error: Option<String>,
    last_cols: u16,
    last_rows: u16,
    pub focus_handle: FocusHandle,
    cell_width: f32,
    cell_height: f32,
    scroll_dirty: std::sync::Arc<std::sync::atomic::AtomicBool>,
    scrollbar_dragging: bool,
    // FPS tracking
    frame_count: u32,
    last_fps_time: Instant,
    pub current_fps: u32,
}

impl TerminalView {
    /// Create a terminal view running a specific command, or default shell if None
    pub fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        command: Option<ShellCommand>,
        working_dir: Option<PathBuf>,
    ) -> Self {
        let focus_handle = cx.focus_handle();

        // Measure cell dimensions from the actual font via GPUI's text system.
        // This value is used both for PTY resize (column/row count) and for
        // the grid element's rendering, ensuring they agree.
        let font = Font {
            family: FONT_FAMILY.into(),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
            features: FontFeatures::disable_ligatures(),
            fallbacks: None,
        };
        let (measured_w, measured_h) =
            TerminalGridElement::measure_cell(window, &font, px(FONT_SIZE));
        let cell_width = f32::from(measured_w);
        let cell_height = f32::from(measured_h);

        let terminal = match PtyTerminal::spawn(TermSize::default(), command, working_dir) {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!("Failed to create PTY: {e}");
                return Self {
                    terminal: None,
                    error: Some(format!("Failed to create PTY: {e}")),
                    last_cols: 80,
                    last_rows: 24,
                    focus_handle,
                    cell_width,
                    cell_height,
                    scroll_dirty: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    scrollbar_dragging: false,
                    frame_count: 0,
                    last_fps_time: Instant::now(),
                    current_fps: 0,
                };
            }
        };

        // Auto-focus this terminal on creation
        focus_handle.focus(window, cx);

        // Poll for PTY events on a timer and re-render
        cx.spawn_in(window, async |this: WeakEntity<Self>, cx: &mut AsyncWindowContext| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;

                let should_redraw = this
                    .update(cx, |this: &mut Self, _cx: &mut Context<Self>| {
                        let pty_events = if let Some(ref mut terminal) = this.terminal {
                            terminal.drain_events()
                        } else {
                            false
                        };
                        // Also check if scroll happened
                        let scrolled = this.scroll_dirty.swap(false, std::sync::atomic::Ordering::Relaxed);
                        pty_events || scrolled
                    })
                    .unwrap_or(false);

                if should_redraw {
                    this.update(cx, |_this: &mut Self, cx: &mut Context<Self>| {
                        cx.notify();
                    })
                    .ok();
                }
            }
        })
        .detach();

        // Observe window bounds changes for resize
        cx.observe_window_bounds(window, |this: &mut Self, window, _cx| {
            let viewport = window.viewport_size();
            let sidebar_width = px(240.0);
            let available_width = viewport.width - sidebar_width;
            let available_height = viewport.height;

            if available_width > px(100.0) && available_height > px(100.0) {
                let new_size = Self::compute_size(
                    f32::from(available_width),
                    f32::from(available_height),
                    this.cell_width,
                    this.cell_height,
                );
                if new_size.cols != this.last_cols || new_size.rows != this.last_rows {
                    this.last_cols = new_size.cols;
                    this.last_rows = new_size.rows;
                    if let Some(ref mut terminal) = this.terminal {
                        terminal.resize(new_size);
                    }
                }
            }
        })
        .detach();

        Self {
            terminal,
            error: None,
            last_cols: 80,
            last_rows: 24,
            focus_handle,
            cell_width,
            cell_height,
            scroll_dirty: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            scrollbar_dragging: false,
            frame_count: 0,
            last_fps_time: Instant::now(),
            current_fps: 0,
        }
    }

    /// Focus this terminal view
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus_handle.focus(window, cx);
    }

    /// Check if the PTY process has exited
    pub fn has_exited(&self) -> bool {
        self.terminal.as_ref().map_or(true, |t| t.exited)
    }

    /// Compute terminal grid size from pixel dimensions
    fn compute_size(width_px: f32, height_px: f32, cell_w: f32, cell_h: f32) -> TermSize {
        let cols = (width_px / cell_w).floor() as u16;
        let rows = (height_px / cell_h).floor() as u16;
        TermSize {
            cols: cols.max(MIN_COLS),
            rows: rows.max(MIN_ROWS),
            cell_width: cell_w as u16,
            cell_height: cell_h as u16,
        }
    }
}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // FPS tracking
        self.frame_count += 1;
        let elapsed = self.last_fps_time.elapsed();
        if elapsed >= Duration::from_secs(1) {
            self.current_fps = self.frame_count;
            self.frame_count = 0;
            self.last_fps_time = Instant::now();
        }

        if let Some(ref error) = self.error {
            return div()
                .size_full()
                .bg(rgb(0x1e1e2e))
                .text_color(rgb(0xf38ba8))
                .child(div().p(px(12.0)).child(error.clone()))
                .into_any_element();
        }

        let Some(ref terminal) = self.terminal else {
            return div()
                .size_full()
                .bg(rgb(0x1e1e2e))
                .child("No terminal")
                .into_any_element();
        };

        let font = Font {
            family: FONT_FAMILY.into(),
            weight: FontWeight::NORMAL,
            style: FontStyle::Normal,
            features: FontFeatures::disable_ligatures(),
            fallbacks: None,
        };

        let grid_element = TerminalGridElement::new(
            terminal.term.clone(),
            font,
            px(FONT_SIZE),
            px(self.cell_width),
            px(self.cell_height),
        );

        div()
            .id("terminal")
            .size_full()
            .bg(rgb(0x1e1e2e))
            .overflow_hidden()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(|this: &mut Self, event: &KeyDownEvent, _window, cx| {
                let key = event.keystroke.key.as_str();
                let mods = &event.keystroke.modifiers;

                // Handle Cmd shortcuts (emit to parent)
                if mods.platform {
                    match key {
                        "n" => { cx.emit(TerminalEvent::NewSession); return; }
                        "w" => { cx.emit(TerminalEvent::CloseSession); return; }
                        "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" => {
                            if let Ok(num) = key.parse::<usize>() {
                                cx.emit(TerminalEvent::SwitchSession(num - 1));
                            }
                            return;
                        }
                        _ => {}
                    }
                }

                let Some(ref terminal) = this.terminal else { return };

                // Handle control key combos
                if mods.control {
                    let ctrl_byte = match key {
                        "a" => Some(0x01), "b" => Some(0x02), "c" => Some(0x03),
                        "d" => Some(0x04), "e" => Some(0x05), "f" => Some(0x06),
                        "g" => Some(0x07), "h" => Some(0x08), "k" => Some(0x0b),
                        "l" => Some(0x0c), "n" => Some(0x0e), "o" => Some(0x0f),
                        "p" => Some(0x10), "r" => Some(0x12), "t" => Some(0x14),
                        "u" => Some(0x15), "w" => Some(0x17), "z" => Some(0x1a),
                        _ => None,
                    };
                    if let Some(byte) = ctrl_byte {
                        terminal.write(&[byte]);
                        return;
                    }
                }

                // Handle special keys BEFORE key_char — enter, backspace, etc.
                let special_bytes: Option<&[u8]> = match key {
                    "enter" => Some(b"\r"),
                    "backspace" => Some(b"\x7f"),
                    "tab" => Some(b"\t"),
                    "escape" => Some(b"\x1b"),
                    "up" => Some(b"\x1b[A"),
                    "down" => Some(b"\x1b[B"),
                    "right" => Some(b"\x1b[C"),
                    "left" => Some(b"\x1b[D"),
                    "home" => Some(b"\x1b[H"),
                    "end" => Some(b"\x1b[F"),
                    "pageup" => Some(b"\x1b[5~"),
                    "pagedown" => Some(b"\x1b[6~"),
                    "delete" => Some(b"\x1b[3~"),
                    "space" => Some(b" "),
                    _ => None,
                };

                if let Some(bytes) = special_bytes {
                    terminal.write(bytes);
                    return;
                }

                // For regular character input, use key_char
                if let Some(ref key_char) = event.keystroke.key_char {
                    terminal.write(key_char.as_bytes());
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this: &mut Self, event: &MouseDownEvent, window, cx| {
                    // Check if click is in scrollbar region (rightmost 12px)
                    let viewport = window.viewport_size();
                    let click_x = f32::from(event.position.x);
                    let scrollbar_zone = f32::from(viewport.width) - 12.0;

                    if click_x >= scrollbar_zone {
                        let Some(ref terminal) = this.terminal else { return };
                        let t = terminal.term.lock();
                        let grid = t.grid();
                        let total = grid.total_lines();
                        let screen = grid.screen_lines();
                        let history = total.saturating_sub(screen);
                        drop(t);

                        if history > 0 {
                            this.scrollbar_dragging = true;
                            // Set scroll position from click y
                            let click_y = f32::from(event.position.y);
                            let viewport_h = f32::from(viewport.height);
                            let fraction = (click_y / viewport_h).clamp(0.0, 1.0);
                            // fraction 0 = top (max offset), fraction 1 = bottom (offset 0)
                            let new_offset = ((1.0 - fraction) * history as f32).round() as i32;
                            let current = terminal.term.lock().grid().display_offset() as i32;
                            let delta = new_offset - current;
                            if delta != 0 {
                                terminal.term.lock().scroll_display(Scroll::Delta(delta));
                                this.scroll_dirty.store(true, std::sync::atomic::Ordering::Relaxed);
                                cx.notify();
                            }
                        }
                    }
                }),
            )
            .on_mouse_move(cx.listener(|this: &mut Self, event: &MouseMoveEvent, window, cx| {
                if !this.scrollbar_dragging { return; }
                let Some(ref terminal) = this.terminal else { return };
                let t = terminal.term.lock();
                let total = t.grid().total_lines();
                let screen = t.grid().screen_lines();
                let history = total.saturating_sub(screen);
                let current_offset = t.grid().display_offset() as i32;
                drop(t);

                if history == 0 { return; }

                let viewport_h = f32::from(window.viewport_size().height);
                let mouse_y = f32::from(event.position.y);
                let fraction = (mouse_y / viewport_h).clamp(0.0, 1.0);
                let new_offset = ((1.0 - fraction) * history as f32).round() as i32;
                let delta = new_offset - current_offset;
                if delta != 0 {
                    terminal.term.lock().scroll_display(Scroll::Delta(delta));
                    this.scroll_dirty.store(true, std::sync::atomic::Ordering::Relaxed);
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this: &mut Self, _event: &MouseUpEvent, _window, _cx| {
                    this.scrollbar_dragging = false;
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this: &mut Self, _event: &MouseUpEvent, _window, _cx| {
                    this.scrollbar_dragging = false;
                }),
            )
            .on_scroll_wheel({
                let term = self.terminal.as_ref().map(|t| t.term.clone());
                let scroll_dirty = self.scroll_dirty.clone();
                let cell_h = self.cell_height;
                move |event: &ScrollWheelEvent, _window: &mut Window, _cx: &mut App| {
                    let Some(ref term) = term else { return };
                    let delta = event.delta.pixel_delta(px(cell_h));
                    let lines = (f32::from(delta.y) / cell_h).round() as i32;
                    if lines != 0 {
                        term.lock().scroll_display(Scroll::Delta(lines));
                        scroll_dirty.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            })
            .child(grid_element)
            .into_any_element()
    }
}
