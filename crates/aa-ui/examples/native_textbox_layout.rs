//! The original speech-bubble constructor and SetChatMsg/AdjustBubbleLayout at native UI scales.
#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;
use aa_ui::UiRuntime;
use bevy::prelude::*;

pub(crate) fn install_at_scale(ui: &mut UiRuntime, scale: f32) {
    fake::install(ui, "en_us", &fake::default_worlds());
    ui.load_base_libs();
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    ui.exec("@textbox_fixture", &format!(r#"
        UIParent:SetUIScale({scale})
        X2Unit.GetUnitId=function() return 'player' end
        X2Unit.GetUnitInfoById=function() return {{faction='friendly',type='npc'}} end
        local positions={{first={{300/{scale},330/{scale},1}},second={{730/{scale},330/{scale},1}},third={{300/{scale},520/{scale},1}}}}
        X2Unit.GetUnitScreenNameTagOffset=function(_,id) return unpack(positions[id]) end
    "#)).unwrap();
    let paths = [
        "scriptsbin/x2ui/chat/locale/ko.alb".into(),
        "scriptsbin/x2ui/chat/locale/en_us.alb".into(),
        "scriptsbin/x2ui/chat/chatbubble.alb".into(),
    ];
    let errors = aa_lua::run_files(&ui.lua, &paths);
    assert!(errors.is_empty(), "{errors:?}");
    ui.exec("@textbox_bubbles", r#"
        assert(FONT_SIZE.LARGE==15 and FONT_SIZE.MIDDLE==13 and chatLocale.lineSpace.chatBubble==3)
        local board=CreateEmptyWindow('textboxBoard','UIParent')
        board:SetUILayer('background');board:AddAnchor('TOPLEFT',UIParent,0,0);board:AddAnchor('BOTTOMRIGHT',UIParent,0,0);board:Show(true)
        local paper=board:CreateColorDrawable(.08,.10,.13,1,'background')
        paper:AddAnchor('TOPLEFT',board,0,0);paper:AddAnchor('BOTTOMRIGHT',board,0,0)
        local function caption(id,text,x,y,width)
            local label=board:CreateChildWidget('label',id,0,true)
            label:SetExtent(width,30);label:AddAnchor('TOPLEFT',board,x,y)
            label.style:SetAlign(ALIGN_LEFT);label.style:SetColor(.88,.88,.83,1);label:SetText(text)
        end
        caption('heading','Original speech bubbles / native physical textbox rows',60,35,1150)
        caption('wrapped','Original SetChatMsg / two rows / LARGE 15',190,175,480)
        caption('long','Original wrapping / longer description',700,175,480)
        caption('short','Original single-row width and height',190,430,480)
        local samples={
            {'first','Watcher','Old forests conceal distant roads and hidden valleys'},
            {'second','River guide','Follow the river toward the eastern watchtower and return before nightfall.'},
            {'third','Scout','Roads are clear.'},
        }
        for _,sample in ipairs(samples) do
            local bubble=chatBubbleFrame:AddBubble(sample[1],true,CHAT_SAY,60000)
            bubble:SetChatMsg(sample[2],sample[3])
            bubble:OnUpdate(0)
        end
    "#).unwrap();
    fake::report(ui, "native_textbox_layout");
}
fn install(ui: &mut UiRuntime) {
    let scale = if std::env::args().any(|a| a == "--scale1") {
        1.0
    } else {
        0.85
    };
    install_at_scale(ui, scale);
}
fn main() {
    App::new()
        .add_plugins((
            aa_dev::default_plugins("aa-ui: original speech-bubble textbox rows"),
            aa_dev::DevPlugin,
        ))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(fake::config()),
        })
        .insert_resource(aa_ui::UiSetup::new(install))
        .add_systems(Startup, fake::physical_pixels)
        .run();
}
