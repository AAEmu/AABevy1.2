//! Original loot link constants, native chat labels and the original quest tooltip.
//! `cargo run -p aa-ui --example native_item_links -- --shot shots/aa-ui/native-item-links.png --shot-after 6`

#[path = "fake/mod.rs"]
#[allow(dead_code)]
mod fake;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(aa_dev::default_plugins("aa-ui: original item and quest links"))
        .add_plugins(aa_dev::DevPlugin)
        .add_plugins(aa_ui::UiPlugin { config: Some(fake::config()) })
        .insert_resource(aa_ui::UiSetup::new(|ui| {
            fake::install(ui, "en_us", &fake::default_worlds());
            fake::install_ingame(ui);
            ui.run_stage(aa_ui::Stage::World);
            fake::enter_world(ui);
            let chat = aa_ui::lua::ui(&ui.lua).tree.of_class("chatwindow")[0];
            ui.lua.globals().set("inputWindow", aa_ui::lua::object_table(&ui.lua, chat).unwrap()).unwrap();
            ui.lua.globals().set("encodeNativeChat", ui.lua.create_function(|_, text: String| {
                Ok(aa_ui::lua::chat::encode_for_transport(&text))
            }).unwrap()).unwrap();
            ui.exec("@original_item_links", r#"
                originalLinks={}
                local original,index
                for i=1,20 do
                    local name,v=debug.getupvalue(DebugShowLootDiceWindow,i)
                    if type(v)=='function' and name=='LootDiceEventHandler' then original,index=v,i; break end
                end
                assert(index~=nil)
                debug.setupvalue(DebugShowLootDiceWindow,index,function(key,time,link) table.insert(originalLinks,link) end)
                DebugShowLootDiceWindow()
                debug.setupvalue(DebugShowLootDiceWindow,index,original)
                -- Read-only compact.sqlite3: localized_texts items/name and item_grades/color_argb.
                local names={[24488]='Crushing Grip',[14698]='Radiant Fire Lunadrop',[24498]="Exiled Wizard's Sash"}
                local colors={[2]={119/255,176/255,100/255,1},[3]={85/255,143/255,215/255,1},[4]={203/255,114/255,216/255,1}}
                AAUI.ChatLink=function(kind,id,grade)
                    if kind=='item' and names[id] then return {name=names[id],color=colors[grade]} end
                    if kind=='quest' then return {name=X2Quest:GetQuestContextMainTitle(id),color={1,1,1,1}} end
                end
                message=UIParent:CreateWidget('message','nativeLinkMessage','UIParent')
                message:AddAnchor('TOPLEFT',UIParent,80,300); message:SetExtent(620,180)
                message:SetInset(7,3,9,5); message.style:SetFontSize(16); message:Show(true)
                AddItemLinkHandlerToMessageWidget(message)
                local lines={
                    '|kSocialpeerone,0;: '..originalLinks[1],
                    '|kSocialpeerone,0;: '..originalLinks[2],
                    '|kSocialpeerone,0;: '..originalLinks[3],
                    'Refone: |q2531;',
                }
                for _,line in ipairs(lines) do
                    message:AddMessage(line)
                    X2Chat:DispatchChatMessage(CMF_SAY,line)
                end
                transportText=encodeNativeChat('Escapes: mail@host @@ |cFFFF0000 |kFake,0; |r '..originalLinks[1]..' |q2531;')
                message:AddMessage('|kSocialpeerone,0;: '..transportText)
                X2Quest.GetLinkedQuestSummary=function(self,id) return X2Quest:GetQuestContextSummary(id) end
                X2Quest.GetLinkedQuestObjectives=function(self,id) return {} end
                ShowQuestLinkTooltip({questType=2531})
                local tip=tooltip.tooltipWindows[8]
                tip:RemoveAllAnchors(); tip:AddAnchor('TOPLEFT',UIParent,760,340)
                local edit=inputWindow:GetChatEdit()
                edit:Show(true); edit:SetText('Link: '); edit:SetFocus()
                AddLinkItem(originalLinks[1])
                X2Chat:AddQuestLinkToActiveChatInput('|q2531;')
                assert(edit:GetText()=='Link: [Crushing Grip][The Prophet Terrien]')
            "#).unwrap();
            let transport: String = ui.lua.globals().get("transportText").unwrap();
            ui.fire_event("CHAT_MESSAGE", (0, "0", false, "Socialpeerone", transport, true, ""));
            fake::report(ui, "native_item_links");
        }))
        .add_systems(Startup, fake::physical_pixels)
        .add_systems(Update, fake::scripted_input)
        .run();
}
