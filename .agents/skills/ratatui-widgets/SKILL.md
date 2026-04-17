---
name: ratatui-widgets
description: |
  CRITICAL: Use for ratatui widgets and UI components. Triggers on:
  Block, Paragraph, List, Table, Tabs, Chart, Gauge, Scrollbar, Canvas,
  Widget, StatefulWidget, ListState, TableState, ListItem, Row, Cell,
  BarChart, Sparkline, LineGauge, Clear, render_widget, render_stateful_widget,
  "custom widget", "create widget", "ratatui widget",
  组件, 控件, 列表, 表格, 进度条, 图表, 自定义组件
---

# Ratatui Widgets Skill

> **Version:** ratatui 0.30.0 | **Last Updated:** 2026-01-17
>
> Check for updates: https://crates.io/crates/ratatui

You are an expert at the Rust `ratatui` crate widgets. Help users by:
- **Writing code**: Generate Rust code following the patterns below
- **Answering questions**: Explain concepts, troubleshoot issues, reference documentation

## Documentation

Refer to the local files for detailed documentation:
- `../../references/widgets/built-in-widgets.md` - All widget types and usage
- `../../references/widgets/custom-widgets.md` - Creating custom widgets
- `../../references/_shared/rust-defaults.md` - Rust code generation defaults

## IMPORTANT: Documentation Completeness Check

**Before answering questions, Claude MUST:**

1. Read the relevant reference file(s) listed above
2. If file read fails or file is empty:
   - Inform user: "本地文档不完整，建议运行 `/sync-crate-skills ratatui --force` 更新文档"
   - Still answer based on SKILL.md patterns + built-in knowledge
3. If reference file exists, incorporate its content into the answer

## Widget Traits

| Trait | Description | Render Method |
|-------|-------------|---------------|
| `Widget` | Stateless, consumed on render | `frame.render_widget(w, area)` |
| `StatefulWidget` | Has external state | `frame.render_stateful_widget(w, area, &mut state)` |
| `WidgetRef` | Render by reference (unstable) | `w.render_ref(area, buf)` |

## Key Patterns

### Pattern 1: Block with Content

```rust
use ratatui::widgets::{Block, Paragraph};

let block = Block::bordered()
    .title("My Block")
    .title_bottom("Footer");

let paragraph = Paragraph::new("Content here")
    .block(block);

frame.render_widget(paragraph, area);
```

### Pattern 2: List with Selection

```rust
use ratatui::widgets::{Block, List, ListItem, ListState};
use ratatui::style::{Style, Stylize};

let items: Vec<ListItem> = vec![
    ListItem::new("Item 1"),
    ListItem::new("Item 2"),
    ListItem::new("Item 3"),
];

let list = List::new(items)
    .block(Block::bordered().title("List"))
    .highlight_style(Style::new().reversed())
    .highlight_symbol("> ");

let mut state = ListState::default();
state.select(Some(0));

frame.render_stateful_widget(list, area, &mut state);
```

### Pattern 3: Table with Selection

```rust
use ratatui::widgets::{Block, Cell, Row, Table, TableState};
use ratatui::style::Stylize;
use ratatui::layout::Constraint;

let rows = vec![
    Row::new(vec![Cell::from("Alice"), Cell::from("25")]),
    Row::new(vec![Cell::from("Bob"), Cell::from("30")]),
];

let table = Table::new(rows, [Constraint::Percentage(70), Constraint::Percentage(30)])
    .block(Block::bordered().title("Users"))
    .header(Row::new(vec!["Name", "Age"]).bold())
    .highlight_style(Style::new().reversed());

let mut state = TableState::default();
state.select(Some(0));

frame.render_stateful_widget(table, area, &mut state);
```

### Pattern 4: Tabs

```rust
use ratatui::widgets::{Block, Tabs};
use ratatui::style::Stylize;

let tabs = Tabs::new(vec!["Tab 1", "Tab 2", "Tab 3"])
    .block(Block::bordered())
    .select(0)
    .highlight_style(Style::new().bold().yellow());

frame.render_widget(tabs, area);
```

### Pattern 5: Gauge/Progress

```rust
use ratatui::widgets::{Block, Gauge};
use ratatui::style::{Color, Style};

let gauge = Gauge::default()
    .block(Block::bordered().title("Progress"))
    .gauge_style(Style::new().fg(Color::Green))
    .percent(75)
    .label("75%");

frame.render_widget(gauge, area);
```

## API Reference Table

| Widget | Key Methods | State Type |
|--------|-------------|------------|
| `Block` | `bordered()`, `title()`, `padding()` | None |
| `Paragraph` | `new()`, `block()`, `wrap()`, `scroll()` | None |
| `List` | `new()`, `highlight_style()`, `highlight_symbol()` | `ListState` |
| `Table` | `new()`, `header()`, `widths()`, `highlight_style()` | `TableState` |
| `Tabs` | `new()`, `select()`, `highlight_style()` | None |
| `Gauge` | `percent()`, `ratio()`, `label()` | None |
| `LineGauge` | `ratio()`, `line_set()` | None |
| `Scrollbar` | `orientation()`, `thumb_symbol()` | `ScrollbarState` |
| `Sparkline` | `data()`, `max()`, `bar_set()` | None |
| `Chart` | `datasets()`, `x_axis()`, `y_axis()` | None |
| `BarChart` | `data()`, `bar_width()`, `bar_gap()` | None |
| `Canvas` | `paint()`, `marker()`, `x_bounds()`, `y_bounds()` | None |
| `Clear` | (none) | None |

## When Writing Code

1. Wrap content widgets with `Block` for borders and titles
2. Use `ListState::default().select(Some(0))` to start with first item selected
3. Navigate stateful widgets by modifying state, not widget
4. Use `Clear` before rendering popups to erase underlying content
5. Implement `Widget for &MyWidget` for reusable custom widgets

## When Answering Questions

1. Widgets are consumed when rendered (use references for reuse)
2. Stateful widgets require external state management
3. Selection uses `Option<usize>` - `None` means nothing selected
4. Most widgets accept `Into<Text>` for content
5. Block is a container widget, not standalone content
