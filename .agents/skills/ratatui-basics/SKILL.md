---
name: ratatui-basics
description: |
  CRITICAL: Use for ratatui basics, terminal initialization, and app structure. Triggers on:
  ratatui, TUI, terminal ui, ratatui::run, ratatui::init, ratatui::restore,
  DefaultTerminal, Frame, terminal.draw, crossterm, termion, termwiz,
  "how to start ratatui", "ratatui hello world", "ratatui app structure",
  ratatui 入门, 终端界面, ratatui 教程, TUI 应用, 如何初始化 ratatui
---

# Ratatui Basics Skill

> **Version:** ratatui 0.30.0 | **Last Updated:** 2026-01-17
>
> Check for updates: https://crates.io/crates/ratatui

You are an expert at the Rust `ratatui` crate. Help users by:
- **Writing code**: Generate Rust code following the patterns below
- **Answering questions**: Explain concepts, troubleshoot issues, reference documentation

## Documentation

Refer to the local files for detailed documentation:
- `../../references/basics/app-structure.md` - Application patterns and event loops
- `../../references/basics/backends.md` - Backend comparison and configuration
- `../../references/_shared/rust-defaults.md` - Rust code generation defaults

## IMPORTANT: Documentation Completeness Check

**Before answering questions, Claude MUST:**

1. Read the relevant reference file(s) listed above
2. If file read fails or file is empty:
   - Inform user: "本地文档不完整，建议运行 `/sync-crate-skills ratatui --force` 更新文档"
   - Still answer based on SKILL.md patterns + built-in knowledge
3. If reference file exists, incorporate its content into the answer

## Key Concepts

Ratatui uses **immediate rendering with intermediate buffers**:
- Each frame, render all widgets to a buffer
- Terminal compares current/previous buffers
- Only changed cells are written to terminal

## Key Patterns

### Pattern 1: Simplest App with `run()`

```rust
use crossterm::event;

fn main() -> std::io::Result<()> {
    ratatui::run(|mut terminal| {
        loop {
            terminal.draw(|frame| {
                frame.render_widget("Hello World!", frame.area());
            })?;
            if event::read()?.is_key_press() {
                break Ok(());
            }
        }
    })
}
```

### Pattern 2: Manual init/restore

```rust
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{DefaultTerminal, Frame};
use ratatui::widgets::Paragraph;

fn main() -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let result = run(&mut terminal);
    ratatui::restore();
    result
}

fn run(terminal: &mut DefaultTerminal) -> std::io::Result<()> {
    loop {
        terminal.draw(render)?;
        if handle_events()? {
            break Ok(());
        }
    }
}

fn render(frame: &mut Frame) {
    frame.render_widget(Paragraph::new("Hello!"), frame.area());
}

fn handle_events() -> std::io::Result<bool> {
    match event::read()? {
        Event::Key(key) if key.kind == KeyEventKind::Press => {
            if key.code == KeyCode::Char('q') {
                return Ok(true);
            }
        }
        _ => {}
    }
    Ok(false)
}
```

### Pattern 3: App Struct Pattern

```rust
use ratatui::{DefaultTerminal, Frame};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::{Paragraph, Widget};

struct App {
    counter: u32,
    should_quit: bool,
}

impl App {
    fn run(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        while !self.should_quit {
            terminal.draw(|frame| frame.render_widget(&*self, frame.area()))?;
            self.handle_events()?;
        }
        Ok(())
    }
}

impl Widget for &App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        Paragraph::new(format!("Count: {}", self.counter)).render(area, buf);
    }
}
```

## API Reference Table

| Function/Type | Description | Example |
|---------------|-------------|---------|
| `ratatui::run(f)` | Run app with auto init/restore | `ratatui::run(\|t\| { ... })` |
| `ratatui::init()` | Initialize terminal | `let mut term = ratatui::init();` |
| `ratatui::restore()` | Restore terminal state | `ratatui::restore();` |
| `DefaultTerminal` | Type alias for default backend | `&mut DefaultTerminal` |
| `terminal.draw(f)` | Draw a frame | `terminal.draw(\|frame\| { ... })?;` |
| `frame.area()` | Get drawable area | `let area = frame.area();` |
| `frame.render_widget(w, a)` | Render widget | `frame.render_widget(para, area);` |

## When Writing Code

1. Use `ratatui::run()` for simple apps - handles init/restore automatically
2. Use separate `init()/restore()` when you need more control
3. Always call `restore()` even on error - use separate function for main loop
4. Handle `KeyEventKind::Press` to avoid duplicate key events on Windows
5. Use `crossterm` backend by default (works on all platforms)

## When Answering Questions

1. Ratatui is immediate mode - rebuild UI every frame
2. Terminal maintains two buffers for differential rendering
3. Event handling is separate from ratatui - use crossterm/termion directly
4. Frame is only valid during the `draw()` closure
5. Widgets are consumed when rendered (implement on `&Widget` for reuse)
