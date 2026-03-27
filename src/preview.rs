use std::io::{Write, stdout};

use clap::ValueEnum;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum PreviewMode {
    Single,
    Line,
    Off,
}

pub struct PreviewPrinter {
    mode: PreviewMode,
    last_text: String,
    last_width: usize,
}

impl PreviewPrinter {
    pub fn new(mode: PreviewMode) -> Self {
        Self {
            mode,
            last_text: String::new(),
            last_width: 0,
        }
    }

    pub fn reset(&mut self) {
        self.last_text.clear();
        self.last_width = 0;
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
                let output = format!("{prefix} {text}");
                let padding = if output.len() < self.last_width {
                    " ".repeat(self.last_width - output.len())
                } else {
                    String::new()
                };
                print!("\r{output}{padding}");
                let _ = stdout().flush();
                self.last_width = output.len();
            }
            PreviewMode::Line => {
                println!("{prefix} {text}");
            }
            PreviewMode::Off => {}
        }

        self.last_text = text.to_owned();
    }

    pub fn finish(&mut self) {
        if matches!(self.mode, PreviewMode::Single) && self.last_width > 0 {
            println!();
        }
        self.reset();
    }
}
