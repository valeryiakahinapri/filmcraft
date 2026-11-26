//! The header bar (Premiere 26 layout, 38 pt): the menu popover's hamburger and the workflow
//! stages on the left, the document title centred, and the quick actions on the right. The stages
//! are the DaVinci Resolve-style page switch — Import, Assemble, Edit, Color, Audio, Overlay,
//! Effects, Export — where the middle ones pick the Edit mode's workspace and Import / Export are
//! their own modes. On macOS the menus also live in the native menu bar (set up by the app);
//! everywhere the hamburger opens them as a popover.

use egui::{Color32, Rect, Sense, Stroke, pos2, vec2};

use crate::FilmcraftApp;
use crate::icons::{self, Icon};
use crate::state::Mode;
use crate::theme::Tokens;

/// Height of the header bar. With `integrated_titlebar` it sits in the macOS title-bar strip.
pub const HEIGHT: f32 = 38.0;

/// A workflow stage: one step of the header's stage switch. The Edit-mode stages are the Edit
/// screens with a workspace picked; Import and Export are their own modes.
struct Stage {
    /// The automation id suffix (`header.stage.<id>`).
    id: &'static str,
    /// The English name: the tooltip, the automation label.
    label: &'static str,
    icon: Icon,
    /// The mode the stage switches to.
    mode: Mode,
    /// The workspace the Edit-mode stages switch to (their layout); `None` on Import / Export.
    workspace: Option<&'static str>,
}

/// The workflow stages, in order: Import, then the rough cut (Assemble), the refine pass (Edit),
/// then the passes that own a panel layout — Colour, Audio, Overlay (captions and graphics),
/// Effects — and finally the delivery (Export).
const STAGES: [Stage; 8] = [
    Stage { id: "import", label: "Import", icon: Icon::Import, mode: Mode::Import, workspace: None },
    Stage { id: "assemble", label: "Assemble", icon: Icon::Film, mode: Mode::Edit, workspace: Some("Assembly") },
    Stage { id: "edit", label: "Edit", icon: Icon::Sequence, mode: Mode::Edit, workspace: Some("Editing") },
    Stage { id: "color", label: "Color", icon: Icon::ColorWheel, mode: Mode::Edit, workspace: Some("Color") },
    Stage { id: "audio", label: "Audio", icon: Icon::Audio, mode: Mode::Edit, workspace: Some("Audio") },
    Stage { id: "overlay", label: "Overlay", icon: Icon::Captions, mode: Mode::Edit, workspace: Some("Captions and Graphics") },
    Stage { id: "effects", label: "Effects", icon: Icon::Fx, mode: Mode::Edit, workspace: Some("Effects") },
    Stage { id: "export", label: "Export", icon: Icon::Export, mode: Mode::Export, workspace: None },
];

impl Stage {
    /// The stage is the one in use: its mode is on and, for the Edit-mode stages, its workspace
    /// (a workspace picked from the Window menu, or a user's own, leaves no stage lit).
    fn is_active(&self, app: &FilmcraftApp) -> bool {
        if app.ui.mode != self.mode {
            return false;
        }
        self.workspace.is_none_or(|w| crate::dock::slug(&app.ui.workspace) == crate::dock::slug(w))
    }
}

/// Draws the header bar in `rect` and handles its clicks: the hamburger that opens the menus, the
/// workflow stage switch, and the quick actions.
pub fn show(app: &mut FilmcraftApp, ui: &mut egui::Ui, rect: Rect) {
    let t = app.tokens;
    let p = ui.painter().clone();
    p.rect_filled(rect, 0.0, t.header_bg);
    p.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, Color32::BLACK));
    let drag = ui.interact(rect, egui::Id::unique("header-drag"), Sense::click_and_drag());
    if drag.drag_started_by(egui::PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if drag.double_clicked() {
        let max = ui.ctx().input(|i| i.viewport().maximized.unwrap_or(false));
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Maximized(!max));
    }
    let mut x = rect.min.x + if app.integrated_titlebar { 104.0 } else { 14.0 };
    // The hamburger: the menus fold into the button at Home's old spot (the shared, styled,
    // keyboard-accessible popover; see [`crate::menus::menu_popover`]). The Home screen itself is
    // the Import stage next to it.
    {
        let spot = Rect::from_center_size(pos2(x + 10.0, rect.center().y), vec2(30.0, 30.0));
        let mut mu = ui.new_child(egui::UiBuilder::new().max_rect(spot).id_salt("menu-popover"));
        crate::menus::menu_popover(app, &mut mu);
        x = spot.max.x + 12.0;
    }
    // Workflow stages (26 pt icon buttons; active = primary icon with a 2 pt underline, the
    // language the text tabs used)
    for st in &STAGES {
        let r = Rect::from_center_size(pos2(x + 13.0, rect.center().y), vec2(26.0, 26.0));
        let resp = ui.interact(r, egui::Id::unique(("header.stage", st.id)), Sense::click()).on_hover_text(crate::i18n::t(st.label));
        app.auto.add(&format!("header.stage.{}", st.id), r, st.label);
        let active = st.is_active(app);
        if resp.hovered() {
            p.rect_filled(r, 4.0, t.hover);
        }
        let col = if active || resp.hovered() { t.tab_text_active } else { t.tab_text };
        icons::paint(&p, r.shrink(5.0), st.icon, col);
        if active {
            p.line_segment([pos2(r.min.x + 3.0, r.max.y + 2.0), pos2(r.max.x - 3.0, r.max.y + 2.0)], Stroke::new(2.0, t.tab_text_active));
        }
        if resp.clicked() {
            app.ui.mode = st.mode;
            // Only a different workspace reloads its layout: clicking the stage in use keeps the
            // panels as they are (and so does coming back from Import / Export).
            if let Some(w) = st.workspace
                && crate::dock::slug(&app.ui.workspace) != crate::dock::slug(w)
            {
                app.set_workspace(w);
            }
        }
        x = r.max.x + 6.0;
    }
    let left_end = x - 6.0;
    // Document title, centred
    let title = if app.session.is_dirty() { tlf!("{name} - Edited", name = app.session.project.name) } else { app.session.project.name.clone() };
    // Right cluster: icons at ~38 pt pitch.
    let mut rx = rect.max.x - 14.0;
    let mut btn = |ui: &mut egui::Ui, icon: Icon, id: &str, tip: &str, app: &mut FilmcraftApp| -> egui::Response {
        let r = Rect::from_center_size(pos2(rx - 14.0, rect.center().y), vec2(28.0, 28.0));
        rx -= 38.0;
        let resp = ui.interact(r, egui::Id::unique(("hdr", id)), Sense::click()).on_hover_text(tip);
        app.auto.add(&format!("header.{id}"), r, tip);
        if resp.hovered() {
            ui.painter().rect_filled(r, 4.0, t.hover);
        }
        icons::paint(ui.painter(), r.shrink(6.0), icon, if resp.hovered() { t.text } else { t.text_dim });
        resp
    };
    if btn(ui, Icon::Fullscreen, "fullscreen", tl!("Full screen"), app).clicked() {
        let fs = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(!fs));
    }
    if btn(ui, Icon::Speaker, "volume", tl!("Volume"), app).clicked() {
        app.ui.status = tl!("Master volume: use the Audio Track Mixer").into();
    }
    if btn(ui, Icon::Search, "search", tl!("Search"), app).clicked() {
        app.show_panel(crate::dock::PanelKind::Effects);
    }
    if btn(ui, Icon::Bell, "notifications", tl!("Progress"), app).clicked() {
        app.ui.mode = Mode::Export;
    }
    // Quick Export: a popup with File Name & Location, a preset list and Export (Premiere 26)
    let qx = rect.max.x - 14.0 - 4.0 * 38.0; // the fifth button from the right
    if btn(ui, Icon::Export, "quickExport", tl!("Quick Export"), app).clicked() {
        app.ui.export.quick_open = !app.ui.export.quick_open;
        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::unique("quick-export-toggled"), true));
    }
    crate::panels::export_mode::quick_export(app, ui.ctx(), pos2(qx - 340.0, rect.max.y + 4.0));
    // Appearance Mode: click for the next one (Auto, Light, Dark); the icon shows the current one.
    let (icon, mode) = match app.session.prefs.appearance.appearance_mode.as_str() {
        "auto" => (Icon::Monitor, tl!("Sync with system")),
        "light" => (Icon::Sun, tl!("Light")),
        _ => (Icon::Moon, tl!("Dark")),
    };
    let tip = format!("{}: {mode}", tl!("Appearance Mode"));
    let appearance = btn(ui, icon, "appearance", &tip, app);
    appearance.context_menu(|ui| crate::panels::settings::appearance_context_menu(app, ui));
    if appearance.clicked() {
        crate::panels::settings::cycle_appearance(app, ui.ctx());
    }
    // Document title, centred in the gap the stages and the quick actions leave. Localized menus
    // and user titles can be wider than English, so use the actual gap.
    {
        let left = left_end + 12.0;
        let right = appearance.rect.min.x - 12.0;
        if right > left + 40.0 {
            let mut job = egui::text::LayoutJob::simple_singleline(title, Tokens::ui(14.0), t.tab_text_active);
            job.wrap.max_width = right - left;
            job.wrap.max_rows = 1;
            let galley = p.layout_job(job);
            let half = galley.size().x / 2.0;
            let center = if rect.center().x - half >= left && rect.center().x + half <= right { rect.center().x } else { (left + right) / 2.0 };
            p.with_clip_rect(Rect::from_min_max(pos2(left, rect.min.y), pos2(right, rect.max.y))).galley(
                pos2(center - half, rect.center().y - galley.size().y / 2.0),
                galley,
                t.tab_text_active,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The stages are ordered Import → Assemble → Edit → Color → Audio → Overlay → Effects →
    /// Export, with the workspace-named ones between Import and Export.
    #[test]
    fn stages_are_ordered_and_mapped() {
        let ids: Vec<&str> = STAGES.iter().map(|s| s.id).collect();
        assert_eq!(ids, ["import", "assemble", "edit", "color", "audio", "overlay", "effects", "export"]);
        assert_eq!(STAGES[0].mode, Mode::Import);
        assert_eq!(STAGES[7].mode, Mode::Export);
        for st in &STAGES[1..7] {
            assert_eq!(st.mode, Mode::Edit, "{} is an Edit-mode stage", st.id);
            assert!(st.workspace.is_some(), "{} picks a workspace", st.id);
        }
        assert_eq!(STAGES[5].workspace, Some("Captions and Graphics"), "Overlay is Captions and Graphics");
        assert_eq!(STAGES[1].workspace, Some("Assembly"), "Assemble is the Assembly layout");
    }

    /// The stage in use lights up: its mode and workspace, and only then; a workspace that is not
    /// a stage's (picked from the Window menu, or the user's own) leaves the Edit stages unlit.
    #[test]
    fn the_active_stage_follows_the_mode_and_the_workspace() {
        let mut app = FilmcraftApp::new(filmcraft_engine::Session::default());
        app.ui.workspace = "Editing".into();
        app.ui.mode = Mode::Edit;
        assert!(STAGES[2].is_active(&app), "Edit with the Editing workspace");
        assert!(!STAGES[1].is_active(&app), "Assemble is not lit on the Editing workspace");
        app.ui.workspace = "Captions and Graphics".into();
        assert!(STAGES[5].is_active(&app), "Overlay is Captions and Graphics");
        assert!(!STAGES[2].is_active(&app));
        app.ui.mode = Mode::Import;
        assert!(STAGES[0].is_active(&app));
        assert!(!STAGES[5].is_active(&app), "an Edit stage is not lit in Import mode");
        app.ui.mode = Mode::Export;
        assert!(STAGES[7].is_active(&app));
        app.ui.mode = Mode::Edit;
        app.ui.workspace = "Vertical".into();
        assert!(!STAGES.iter().skip(1).take(6).any(|s| s.is_active(&app)), "no stage owns the Vertical workspace");
        // the match is by slug, as the Window menu's ids are
        app.ui.workspace = "captions andgraphics".into();
        assert!(STAGES[5].is_active(&app), "the workspace match ignores spaces and case");
    }

    #[test]
    fn workspace_name_uppercasing_is_turkish_aware() {
        crate::i18n::set_current(crate::i18n::Language::Tr);
        assert_eq!(crate::i18n::to_uppercase("Birleştirme"), "BİRLEŞTİRME");
        assert_eq!(crate::i18n::to_uppercase("Altyazılar ve Grafikler"), "ALTYAZILAR VE GRAFİKLER");
        crate::i18n::set_current(crate::i18n::Language::En);
        assert_eq!(crate::i18n::to_uppercase("Birleştirme"), "BIRLEŞTIRME");
    }
}
