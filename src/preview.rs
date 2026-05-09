use std::io::{Write, stdout};

use clap::ValueEnum;
use terminal_size::{Width, terminal_size};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PreviewMode {
    Single,
    Line,
    Off,
}

pub struct PreviewPrinter {
    mode: PreviewMode,
    last_text: String,
    last_display_width: usize,
}

impl PreviewPrinter {
    pub fn new(mode: PreviewMode) -> Self {
        Self {
            mode,
            last_text: String::new(),
            last_display_width: 0,
        }
    }

    pub fn reset(&mut self) {
        self.last_text.clear();
        self.last_display_width = 0;
    }

    pub fn show(&mut self, text: &str, is_final: bool) {
        if matches!(self.mode, PreviewMode::Off) || text.trim().is_empty() {
            return;
        }
        if !is_final && text == self.last_text {
            return;
        }

        let prefix = if is_final { "[VOICE FINAL]" } else { "[VOICE]" };
        match self.mode {
            PreviewMode::Single => {
                let cleaned_text = text.replace('\r', " ").replace('\n', " ");
                let mut output = format!("{prefix} {cleaned_text}");
                output = shorten_to_terminal_width(&output);

                let output_display_width = output.width();
                let padding = if output_display_width < self.last_display_width {
                    " ".repeat(self.last_display_width - output_display_width)
                } else {
                    String::new()
                };
                print!("\r{output}{padding}");
                let _ = stdout().flush();
                self.last_display_width = output_display_width;
            }
            PreviewMode::Line => {
                println!("{prefix} {text}");
            }
            PreviewMode::Off => {}
        }

        self.last_text = text.to_owned();
    }

    pub fn finish(&mut self) {
        if matches!(self.mode, PreviewMode::Single) && self.last_display_width > 0 {
            println!();
        }
        self.reset();
    }
}

fn shorten_to_terminal_width(line: &str) -> String {
    let Some((Width(cols), _)) = terminal_size().filter(|(Width(cols), _)| *cols > 0) else {
        return line.to_owned();
    };
    let max_cols = cols as usize;
    // Avoid rendering into the right-most column to prevent a "pending wrap" state on some terminals.
    let target_cols = max_cols.saturating_sub(1);
    if target_cols == 0 {
        return String::new();
    }

    if line.width() <= target_cols {
        return line.to_owned();
    }

    // Keep a one-line display by truncating the tail and adding an ellipsis.
    let ellipsis = "...";
    let ellipsis_width = ellipsis.width();
    if target_cols <= ellipsis_width {
        return truncate_to_display_width(line, target_cols);
    }

    let allowed = target_cols - ellipsis_width;
    let mut out = truncate_to_display_width(line, allowed);
    out.push_str(ellipsis);
    out
}

fn truncate_to_display_width(s: &str, max_cols: usize) -> String {
    if max_cols == 0 {
        return String::new();
    }

    let mut out = String::new();
    let mut used = 0usize;
    for ch in s.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > max_cols {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}
