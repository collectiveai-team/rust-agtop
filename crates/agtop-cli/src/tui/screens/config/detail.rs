//! Routes to the active section's render function.

use ratatui::{layout::Rect, Frame};

use crate::tui::msg::ConfigSection;
use crate::tui::screens::config::sections;
use crate::tui::theme_v2::Theme;

#[derive(Debug, Default)]
pub struct DetailModel {
    pub appearance: sections::appearance::AppearanceModel,
    pub columns: sections::columns::ColumnsModel,
    pub refresh: sections::refresh::RefreshModel,
    pub clients: sections::clients::ClientsModel,
    pub data_sources: sections::data_sources::DataSourcesModel,
    pub about: sections::about::AboutModel,
}

pub fn render(
    frame: &mut Frame<'_>,
    area: Rect,
    current: ConfigSection,
    m: &DetailModel,
    theme: &Theme,
) {
    match current {
        ConfigSection::Appearance => {
            sections::appearance::render(frame, area, &m.appearance, theme)
        }
        ConfigSection::Columns => sections::columns::render(frame, area, &m.columns, theme),
        ConfigSection::Refresh => sections::refresh::render(frame, area, &m.refresh, theme),
        ConfigSection::Clients => sections::clients::render(frame, area, &m.clients, theme),
        ConfigSection::Keybinds => sections::keybinds::render(frame, area, &(), theme),
        ConfigSection::DataSources => {
            sections::data_sources::render(frame, area, &m.data_sources, theme)
        }
        ConfigSection::About => sections::about::render(frame, area, &m.about, theme),
    }
}
