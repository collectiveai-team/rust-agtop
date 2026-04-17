---
name: ratatui-styling
description: |
  CRITICAL: Use for ratatui styling, colors, and text. Triggers on:
  Style, Color, Modifier, Stylize, Span, Line, Text,
  fg, bg, bold, italic, underline, red, green, blue, yellow,
  "ratatui color", "ratatui style", "styled text", "text styling",
  样式, 颜色, 字体, 加粗, 斜体, 下划线, ratatui 颜色, ratatui 样式
---

# Ratatui Styling Skill

> **Version:** ratatui 0.30.0 | **Last Updated:** 2026-01-17
>
> Check for updates: https://crates.io/crates/ratatui

You are an expert at the Rust `ratatui` crate styling system. Help users by:
- **Writing code**: Generate Rust code following the patterns below
- **Answering questions**: Explain concepts, troubleshoot issues, reference documentation

## Documentation

Refer to the local files for detailed documentation:
- `../../references/styling/colors.md` - Color types and usage
- `../../references/styling/text-styling.md` - Text primitives and styling patterns
- `../../references/_shared/rust-defaults.md` - Rust code generation defaults

## IMPORTANT: Documentation Completeness Check

**Before answering questions, Claude MUST:**

1. Read the relevant reference file(s) listed above
2. If file read fails or file is empty:
   - Inform user: "本地文档不完整，建议运行 `/sync-crate-skills ratatui --force` 更新文档"
   - Still answer based on SKILL.md patterns + built-in knowledge
3. If reference file exists, incorporate its content into the answer

## Text Hierarchy

```
Text (multiple lines)
 └── Line (single line)
      └── Span (styled segment)
```

## Key Patterns

### Pattern 1: Stylize Trait (Shorthand)

```rust
use ratatui::style::Stylize;

// On strings - returns Span
let span = "Hello".red().bold();

// On widgets
let para = Paragraph::new("Content").blue().on_white();

// Chained styles
let styled = "Text"
    .red()
    .on_black()
    .bold()
    .italic()
    .underlined();
```

### Pattern 2: Explicit Style Struct

```rust
use ratatui::style::{Color, Modifier, Style};

let style = Style::new()
    .fg(Color::Green)
    .bg(Color::Black)
    .add_modifier(Modifier::BOLD | Modifier::ITALIC);

let span = Span::styled("Styled text", style);
```

### Pattern 3: Multi-Styled Line

```rust
use ratatui::text::{Line, Span};
use ratatui::style::Stylize;

let line = Line::from(vec![
    Span::raw("Normal "),
    Span::styled("bold", Style::new().bold()),
    " and ".into(),
    "red".red(),
]);
```

### Pattern 4: Multi-Line Text

```rust
use ratatui::text::Text;

// From string
let text = Text::raw("Line 1\nLine 2\nLine 3");

// From lines
let text = Text::from(vec![
    Line::from("First line".blue()),
    Line::from("Second line".green()),
]);

// With overall style
let text = Text::from("Content").style(Style::new().italic());
```

### Pattern 5: Color Types

```rust
use ratatui::style::Color;

// Named colors
let c = Color::Red;
let c = Color::LightBlue;

// Indexed (256 colors)
let c = Color::Indexed(208);  // Orange

// RGB
let c = Color::Rgb(255, 128, 0);

// Reset to terminal default
let c = Color::Reset;
```

## API Reference Table

| Method | Description | Example |
|--------|-------------|---------|
| `.fg(color)` | Foreground color | `.fg(Color::Red)` |
| `.bg(color)` | Background color | `.bg(Color::Blue)` |
| `.add_modifier(m)` | Add text modifier | `.add_modifier(Modifier::BOLD)` |
| `.remove_modifier(m)` | Remove modifier | `.remove_modifier(Modifier::BOLD)` |
| `.red()` | Red foreground | `"text".red()` |
| `.on_blue()` | Blue background | `"text".on_blue()` |
| `.bold()` | Bold text | `"text".bold()` |
| `.italic()` | Italic text | `"text".italic()` |
| `.underlined()` | Underlined text | `"text".underlined()` |
| `.reversed()` | Reverse fg/bg | `"text".reversed()` |

## Modifier Flags

| Modifier | Description |
|----------|-------------|
| `BOLD` | Bold/bright text |
| `DIM` | Dimmed text |
| `ITALIC` | Italic text |
| `UNDERLINED` | Underlined text |
| `SLOW_BLINK` | Slow blinking |
| `RAPID_BLINK` | Rapid blinking |
| `REVERSED` | Swap fg/bg |
| `HIDDEN` | Hidden text |
| `CROSSED_OUT` | Strikethrough |

## Named Colors

| Basic | Light |
|-------|-------|
| `Black` | `LightBlack` (Gray) |
| `Red` | `LightRed` |
| `Green` | `LightGreen` |
| `Yellow` | `LightYellow` |
| `Blue` | `LightBlue` |
| `Magenta` | `LightMagenta` |
| `Cyan` | `LightCyan` |
| `White` | `LightWhite` (Bright White) |

## When Writing Code

1. Use Stylize trait shortcuts for concise styling
2. Use `Style::new()` when storing/reusing styles
3. Combine modifiers with `|`: `Modifier::BOLD | Modifier::ITALIC`
4. Use `Color::Reset` to restore terminal defaults
5. RGB colors require terminal support (most modern terminals)

## When Answering Questions

1. Stylize trait is auto-imported via prelude
2. Strings styled with Stylize become Span
3. Style can be applied to widgets with `.style()` method
4. Modifiers are bitflags - combine with `|`
5. Underline color requires `underline-color` feature (default on)
