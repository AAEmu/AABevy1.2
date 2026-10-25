//! Original InitEdit/InitMultiline caret endpoint diagnostic; no network or OS clipboard.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use aa_ui::{UiRuntime, lua};
use bevy::prelude::*;

fn install(ui: &mut UiRuntime) {
    fake::install(ui, "en_us", &fake::default_worlds());
    ui.load_base_libs();
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    let scale = if std::env::args().any(|s| s == "--scale85") {
        0.85
    } else {
        1.0
    };
    ui.exec("@original_caret_board", &format!(r#"
        UIParent:SetUIScale({scale})
        board=UIParent:CreateWidget('window','caretBoard','UIParent')
        board:SetExtent(1000,480);board:AddAnchor('CENTER',UIParent,0,0);board:Show(true)
        local paper=board:CreateColorDrawable(.93,.91,.85,1,'background')
        paper:AddAnchor('TOPLEFT',board,0,0);paper:AddAnchor('BOTTOMRIGHT',board,0,0)
        local function caption(id,text,y)
          local label=board:CreateChildWidget('label',id,0,true)
          label:SetExtent(900,32);label:AddAnchor('TOPLEFT',board,40,y)
          label.style:SetAlign(ALIGN_LEFT);label.style:SetColor(.15,.15,.15,1);label:SetText(text)
        end
        caption('heading','Original InitEdit / InitMultiline - caret endpoint geometry',24)
        caption('singleLabel','Single-line input, original brown skin',78)
        single=UIParent:CreateWidget('editbox','originalSingleCaret',board)
        single:SetExtent(900,40);single:AddAnchor('TOPLEFT',board,40,118)
        InitEdit(single);ApplyEditSkin(single,'brown',false)
        single:SetText('ArcheAge 1.2 - original edit skin');single:Show(true)
        caption('multiLabel','Multiline input, original initialization and text insets',188)
        multi=UIParent:CreateWidget('editboxmultiline','originalMultiCaret',board)
        multi:SetExtent(900,150);multi:AddAnchor('TOPLEFT',board,40,228)
        InitMultiline(multi);ApplyEditSkin(multi,'brown',false)
        multi:SetText('Dear friend,\nThe cursor follows the original vertical endpoint settings.');multi:Show(true)
        caption('details','Offset -2 px changes the top; height +2 px changes the bottom. UI scale: {scale}',410)
    "#)).unwrap();
    let field = if std::env::args().any(|s| s == "--multiline") {
        "multi"
    } else {
        "single"
    };
    let value = ui.lua.load(field).eval().unwrap();
    ui.set_focus(lua::value_id(&ui.lua, &value));
    // A native editable character boundary without a selected range.
    ui.key(&aa_ui::UiKey::End);
    ui.key(&aa_ui::UiKey::Left);
    fake::report(ui, "native_edit_caret");
}

// Hold the existing caret's visible phase for a deterministic unattended screenshot.
fn visible(runtime: Option<NonSend<UiRuntime>>) {
    let Some(runtime) = runtime else { return };
    let mut u = lua::ui(&runtime.lua);
    u.caret_t0 = u.time_ms;
    if !u.caret_visible {
        u.caret_visible = true;
        u.tree.repaint();
    }
}

fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: original edit caret"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(install))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, visible.before(aa_ui::UiFrame))
        .run();
}
