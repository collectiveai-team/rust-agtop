---
name: ratatui-layout
description: |
  CRITICAL: Use for ratatui layout and positioning. Triggers on:
  Layout, Constraint, Rect, Flex, Direction, Margin, Alignment,
  horizontal, vertical, areas, split, Length, Percentage, Ratio, Fill, Min, Max,
  "how to split screen", "ratatui layout", "divide terminal",
  布局, 约束, 分割屏幕, ratatui 布局, 水平布局, 垂直布局
---

# Ratatui Layout Skill

> **Version:** ratatui 0.30.0 | **Last Updated:** 2026-01-17
>
> Check for updates: https://crates.io/crates/ratatui

You are an expert at the Rust `ratatui` crate layout system. Help users by:
- **Writing code**: Generate Rust code following the patterns below
- **Answering questions**: Explain concepts, troubleshoot issues, reference documentation

## Documentation

Refer to the local files for detailed documentation:
- `../../references/layout/constraints.md` - Detailed constraint types and priority
- `../../references/layout/flex-modes.md` - Flex distribution examples
- `../../references/_shared/rust-defaults.md` - Rust code generation defaults

## IMPORTANT: Documentation Completeness Check

**Before answering questions, Claude MUST:**

1. Read the relevant reference file(s) listed above
2. If file read fails or file is empty:
   - Inform user: "本地文档不完整，建议运行 `/sync-crate-skills ratatui --force` 更新文档"
   - Still answer based on SKILL.md patterns + built-in knowledge
3. If reference file exists, incorporate its content into the answer

## Key Concepts

- Layout uses **Cassowary constraint solver** algorithm
- Coordinate system: origin (0,0) at top-left, x→right, y→down
- All areas are `Rect` with (x, y, width, height)

## Key Patterns

### Pattern 1: Basic Vertical Split

```rust
use ratatui::layout::{Constraint, Layout};

let [header, body, footer] = Layout::vertical([
    Constraint::Length(3),    // Fixed 3 rows
    Constraint::Fill(1),      // Fill remaining
    Constraint::Length(1),    // Fixed 1 row
])
.areas(frame.area());
```

### Pattern 2: Basic Horizontal Split

```rust
use ratatui::layout::{Constraint, Layout};

let [sidebar, main] = Layout::horizontal([
    Constraint::Length(20),   // Fixed 20 columns
    Constraint::Fill(1),      // Fill remaining
])
.areas(frame.area());
```

### Pattern 3: Nested Layout

```rust
fn complex_layout(area: Rect) -> (Rect, Rect, Rect, Rect) {
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Fill(1),
        Constraint::Length(1),
    ]).areas(area);

    let [sidebar, main] = Layout::horizontal([
        Constraint::Percentage(25),
        Constraint::Fill(1),
    ]).areas(body);

    (header, sidebar, main, footer)
}
```

### Pattern 4: Centered Content

```rust
use ratatui::layout::{Constraint, Flex, Layout};

fn center_rect(area: Rect, width: u16, height: u16) -> Rect {
    let [_, center, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(height),
        Constraint::Fill(1),
    ]).areas(area);

    let [_, center, _] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(width),
        Constraint::Fill(1),
    ]).areas(center);

    center
}
```

### Pattern 5: Using Flex

```rust
use ratatui::layout::{Constraint, Flex, Layout};

// Space between items
let areas = Layout::horizontal([
    Constraint::Length(10),
    Constraint::Length(10),
    Constraint::Length(10),
])
.flex(Flex::SpaceBetween)
.areas(frame.area());
```

## API Reference Table

| Type/Method | Description | Example |
|-------------|-------------|---------|
| `Layout::vertical([...])` | Create vertical layout | `Layout::vertical([Length(3), Fill(1)])` |
| `Layout::horizontal([...])` | Create horizontal layout | `Layout::horizontal([Percentage(50); 2])` |
| `.areas(rect)` | Split and return array | `let [a, b] = layout.areas(area);` |
| `.split(rect)` | Split and return `Rc<[Rect]>` | `let areas = layout.split(area);` |
| `.margin(n)` | Add uniform margin | `.margin(2)` |
| `.horizontal_margin(n)` | Add horizontal margin | `.horizontal_margin(1)` |
| `.vertical_margin(n)` | Add vertical margin | `.vertical_margin(1)` |
| `.spacing(n)` | Space between segments | `.spacing(1)` |
| `.flex(flex)` | Set flex mode | `.flex(Flex::Center)` |

## Constraint Types (Priority Order)

| Constraint | Description | Priority |
|------------|-------------|----------|
| `Min(n)` | At least n cells | Highest |
| `Max(n)` | At most n cells | High |
| `Length(n)` | Exactly n cells | Medium |
| `Percentage(n)` | n% of available | Medium |
| `Ratio(a, b)` | a/b of available | Medium |
| `Fill(n)` | Fill with weight n | Lowest |

## When Writing Code

1. Use `areas()` with destructuring for compile-time known layouts
2. Use `split()` when layout count is dynamic
3. Prefer `Fill(1)` over `Percentage(100)` for flexible areas
4. Use `Length()` for fixed-size elements (headers, footers)
5. `Min/Max` are constraints on the solver, not guarantees

## When Answering Questions

1. Layout is constraint-based using Cassowary algorithm
2. Constraints are resolved by priority (Min > Max > Length/Percentage/Ratio > Fill)
3. All coordinates are `u16` values
4. `Rect::default()` is (0, 0, 0, 0) - useful for hidden areas
5. Margin reduces available space before splitting
