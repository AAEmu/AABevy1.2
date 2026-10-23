//! Unchanged original World/ui_management bytecode; only its X2 storage boundary is a fixture.
#[path = "../examples/native_chat_alpha.rs"]
#[allow(dead_code)]
mod original;

use aa_ui::{UiRuntime, input, lua};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@original_bound_changed", code).expect(code);
}

fn setup() -> UiRuntime {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    original::install(&mut ui);
    run(
        &ui,
        r#"
        store={};saves={};events={}
        X2.GetCharacterUiData=function(self,key) return store[key] end
        X2.SetCharacterUiData=function(self,key,value)
            store[key]=value;table.insert(saves,key)
        end
        p=UIParent:CreateWidget('window','boundParent','UIParent')
        p:SetExtent(100,60);p:AddAnchor('TOPLEFT',UIParent,100,100);p:Show(true)
        c=UIParent:CreateWidget('window','boundDependent','UIParent')
        c:AddAnchor('TOPLEFT',p,0,0);c:AddAnchor('BOTTOMRIGHT',p,0,0)
        AddUISaveHandlerByKey('p',p);AddUISaveHandlerByKey('c',c)
        p.isMoved=true;c.isMoved=true
        p.ProcChangedAnchor=function(self) table.insert(events,'p32') end
        c.ProcChangedAnchor=function(self) table.insert(events,'c32') end
        p.ProcBoundChanged=function(self) table.insert(events,'p33') end
        c.ProcBoundChanged=function(self) table.insert(events,'c33') end
        function expect(value)
            assert(table.concat(events,',')==value,table.concat(events,','));events={}
        end
    "#,
    );
    ui
}

#[test]
fn original_save_closures_see_synchronous_source_and_dependent_bounds_in_native_order() {
    let ui = setup();
    run(
        &ui,
        r#"
        p:SetWidth(150)
        expect('c32,p32,p33,c33')
        assert(table.concat(saves,',')=='ui_bound_c,ui_bound_p,ui_bound_p,ui_bound_c')
        assert(store.ui_bound_c.bound.width==150 and store.ui_bound_p.bound.width==150)
        saves={};p:SetWidth(150)
        expect('c32,p32,p33,c33');assert(#saves==4)
        p:SetWidth(160,false);expect('');assert(#saves==4)
        assert(p:GetWidth()==160 and c:GetWidth()==160)
        p:SetHeight(60);expect('c32,p32,p33,c33')
        assert(store.ui_bound_c.bound.width==160)
    "#,
    );
    lua::tick(&ui.lua, 0.);
    run(&ui, "expect('');assert(#saves==8)");
}

#[test]
fn original_move_and_scale_save_through_anchor_and_completion_without_delayed_bound_events() {
    let ui = setup();
    run(
        &ui,
        r#"
        p.ProcMovedSize=function() table.insert(events,'move') end
        p:AddAnchor('TOPLEFT',UIParent,120,130);expect('c32,p32')
        p:RemoveAllAnchors();expect('')
        p:AddAnchor('TOPLEFT',UIParent,120,130);expect('c32,p32')
        p:SetScale(1);expect('c32,p32')
        p:SetScale(2);expect('c32,p32')
        p:SetScale(1);expect('c32,p32')
        p:StartMoving();expect('c32,p32')
        saves={}
    "#,
    );
    let mut mouse = input::MouseState::default();
    input::update(
        &ui.lua,
        &mut mouse,
        Some((10., 20.)),
        false,
        false,
        false,
        0.,
    );
    run(
        &ui,
        "expect('c32,p32');assert(#saves==2);p:StopMovingOrSizing();expect('move');assert(#saves==3)",
    );
    lua::tick(&ui.lua, 0.);
    run(&ui, "expect('');assert(#saves==3)");
}

#[test]
fn reset_inside_original_bound_continuation_retires_source_without_a_stale_callback() {
    let ui = setup();
    ui.lua
        .globals()
        .set(
            "resetChats",
            ui.lua
                .create_function(|lua, ()| Ok(lua::chat::reset_windows(lua)))
                .unwrap(),
        )
        .unwrap();
    run(
        &ui,
        r#"
        -- Original first-window save registration is delayed until its first OnScale.
        nativeAlphaChat:OnScale()
        nativeAlphaChat.isMoved=true
        nativeAlphaChat.ProcBoundChanged=function(self)
            assert(self==nativeAlphaChat);table.insert(events,'reset');resetChats()
        end
        events={};saves={}
        nativeAlphaChat:SetWidth(460)
        expect('reset')
        assert(#saves==2)
        assert(store['ui_bound_chatWindow[0]'].bound.width==460)
    "#,
    );
    assert!(lua::ui(&ui.lua).tree.of_class("chatwindow").is_empty());
    lua::tick(&ui.lua, 0.);
    run(&ui, "expect('');assert(#saves==2)");
}

#[test]
fn original_general_skill_columns_reflow_inside_the_parent_size_setter() {
    let ui = setup();
    run(
        &ui,
        r#"
        general=CreateGeneralSkillWindow('boundGeneralProbe',p)
        general:SetExtent(1220,300)
        local labels=general.child[1].contentWnd.labels
        assert(#labels>=2)
        local ax,ay=labels[1]:GetEffectiveOffset()
        local bx,by=labels[2]:GetEffectiveOffset()
        assert(bx>ax and ay==by,'original wide columns were not laid out synchronously')
        general:SetWidth(620)
        ax,ay=labels[1]:GetEffectiveOffset();bx,by=labels[2]:GetEffectiveOffset()
        assert(bx==ax and by>ay,'original narrow columns did not reflow synchronously')
    "#,
    );
}
