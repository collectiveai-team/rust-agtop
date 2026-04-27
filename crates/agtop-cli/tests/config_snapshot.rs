mod snapshot_helpers;

use ratatui::layout::Rect;

use agtop_cli::tui::msg::ConfigSection;
use agtop_cli::tui::screens::config::ConfigState;
use agtop_cli::tui::theme_v2::vscode_dark_plus;

use snapshot_helpers::{buffer_to_text, render_to_buffer};

fn snapshot_section(section: ConfigSection, nerd_font: bool, name: &str) {
    let theme = vscode_dark_plus::theme();
    let mut state = ConfigState::default();
    state.current_section = section;
    state.nerd_font = nerd_font;
    let buf = render_to_buffer(140, 30, |f| state.render(f, Rect::new(0, 0, 140, 30), &theme));
    insta::assert_snapshot!(name, buffer_to_text(&buf));
}

#[test] fn config_appearance_nf_off() { snapshot_section(ConfigSection::Appearance, false, "config_appearance_nf_off"); }
#[test] fn config_appearance_nf_on()  { snapshot_section(ConfigSection::Appearance, true,  "config_appearance_nf_on"); }
#[test] fn config_columns_nf_off()    { snapshot_section(ConfigSection::Columns,    false, "config_columns_nf_off"); }
#[test] fn config_refresh_nf_off()    { snapshot_section(ConfigSection::Refresh,    false, "config_refresh_nf_off"); }
#[test] fn config_clients_nf_off()    { snapshot_section(ConfigSection::Clients,    false, "config_clients_nf_off"); }
#[test] fn config_keybinds_nf_off()   { snapshot_section(ConfigSection::Keybinds,   false, "config_keybinds_nf_off"); }
#[test] fn config_data_sources_nf_off() { snapshot_section(ConfigSection::DataSources, false, "config_data_sources_nf_off"); }
#[test] fn config_about_nf_off()      { snapshot_section(ConfigSection::About,      false, "config_about_nf_off"); }
