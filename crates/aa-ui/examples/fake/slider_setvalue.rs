//! Complete original Sound Init with call observation, not replacement handlers.
use aa_ui::{UiRuntime, lua};

#[path = "mod.rs"]
#[allow(dead_code)]
mod fake;

/// Construct the complete original Options frame, before its lazy ShowProc/Init.
pub fn install(ui: &mut UiRuntime) {
    fake::install(ui, "en_us", &fake::default_worlds());
    fake::install_ingame(ui);
    ui.load_base_libs();
    assert!(ui.errors.is_empty(), "ADMISSION: original base scripts {:?}", ui.errors);
    ui.exec("@setvalue_original_constructor", r#"
        assert(type(CreateOptionFrame)=='function' and type(CreateSoundOptionFrame)=='function')
        assert(type(debug.sethook)=='function' and type(debug.getinfo)=='function')
        local ids={OPTION_ITEM_MASTER_VOLUME,OPTION_ITEM_BGM_VOLUME,OPTION_ITEM_SFX_VOLUME,
                   OPTION_ITEM_CINEMA_VOLUME,OPTION_ITEM_USER_MUSIC_VOLUME}
        assert(ids[1]==35 and ids[2]==36 and ids[3]==37 and ids[4]==39 and ids[5]==40)
        for _,id in ipairs(ids) do X2Option:SetItemFloatValue(id,1) end
        X2Option:SetItemFloatValue(OPTION_ITEM_SOUND_QUALITY,2)
        X2Option:SetItemFloatValue(OPTION_ITEM_USE_COMBAT_SOUND,0)
        X2Option:SetItemFloatValue(OPTION_ITEM_USER_MUSIC_DISABLE_OTHERS,0)
        optionFrame=CreateOptionFrame()
        local pages=optionFrame.contentWindow.pageWindow.subFrames
        local sound=pages[3]
        assert(sound and type(sound.Init)=='function')
        local rows=sound.content.optionFrames
        assert(#rows==8)
        local spec={{'master',35,2,151},{'bgm',36,3,173},{'sfx',37,5,180},
                    {'cinema',39,6,185},{'user_music',40,7,190}}
        sliderSetValueProbe={sound=sound,channels={},trace={},channelWrites={},channelProviderReturns={},
                             allWrites={},allProviderReturns={},initial={},byId={}}
        local p=sliderSetValueProbe
        for i,s in ipairs(spec) do
            local c=rows[s[3]].optionControl
            assert(c==rows[s[3]].sliderbar and type(c.Init)=='function' and type(c.SetVolume)=='function')
            local lo,hi=c:GetMinMaxValues()
            assert(lo==0 and hi==10 and c:GetValue()==10)
            assert(GetOptionItemValue(s[2])==1)
            local w,h=c:GetExtent();assert(w>0 and h>0)
            local initInfo=debug.getinfo(c.Init,'S')
            local volumeInfo=debug.getinfo(c.SetVolume,'S')
            assert(initInfo.what=='Lua' and initInfo.linedefined==71)
            assert(volumeInfo.what=='Lua' and volumeInfo.linedefined==60)
            assert(string.find(initInfo.source,'sound_option',1,true))
            p.channels[i]={name=s[1],id=s[2],control=c,init=c.Init,setVolume=c.SetVolume,
                           handlerLine=s[4]}
            p.byId[s[2]]=p.channels[i]
            p.initial[i]={s[2],c:GetValue(),GetOptionItemValue(s[2]),w,h}
        end
        assert(rows[1].optionControl==rows[1].sliderbar)
        assert(rows[4].optionControl and rows[8].optionControl)
        p.nonChannelIds={OPTION_ITEM_SOUND_QUALITY,OPTION_ITEM_USE_COMBAT_SOUND,
                         OPTION_ITEM_USER_MUSIC_DISABLE_OTHERS}
        assert(type(X2Option.SetItemFloatValue)=='function' and type(SetOptionItemValue)=='function')
        p.provider=X2Option.SetItemFloatValue
        p.setOptionItemValue=SetOptionItemValue
        assert(debug.getinfo(p.provider,'S').what=='Lua')
        assert(debug.getinfo(p.setOptionItemValue,'S').what=='Lua')
        -- Both provider functions stay installed unchanged. The call hook observes
        -- the existing fake provider's real storage write/return, not an audio backend.
    "#).expect("ADMISSION: full original Options/Sound construction and five actual providers");
    let probe: mlua::Table = ui.lua.globals().get("sliderSetValueProbe").unwrap();
    let channels: mlua::Table = probe.get("channels").unwrap();
    let registry = lua::reg(&ui.lua).handlers.clone();
    probe.set("registry", registry.clone()).unwrap();
    for i in 1..=5 {
        let channel: mlua::Table = channels.raw_get(i).unwrap();
        let control: mlua::Table = channel.get("control").unwrap();
        let id = lua::value_id(&ui.lua, &mlua::Value::Table(control)).unwrap();
        assert!(lua::ui(&ui.lua).tree.get(id).alive);
        assert!(lua::ui(&ui.lua).tree.widget(id).unwrap().slider.is_some());
        let handlers: mlua::Table = registry.raw_get(id).unwrap();
        let handler: mlua::Function = handlers.raw_get("OnSliderChanged").unwrap();
        channel.set("widgetId", id).unwrap();
        channel.set("handler", handler).unwrap();
    }
    ui.exec("@setvalue_original_handler_admission", r#"
        local p=sliderSetValueProbe
        for _,ch in ipairs(p.channels) do
            local info=debug.getinfo(ch.handler,'S')
            assert(info.what=='Lua' and info.linedefined==ch.handlerLine)
            assert(string.find(info.source,'sound_option',1,true))
            assert(p.registry[ch.widgetId].OnSliderChanged==ch.handler)
        end
        assert(#p.trace==0 and #p.channelWrites==0 and #p.channelProviderReturns==0)
    "#).expect("ADMISSION: actual original handlers remain installed");
}

/// Observe the actual original functions by identity; do not replace any handler.
pub fn init(ui: &UiRuntime) {
    ui.exec("@setvalue_observed_original_init", r#"
        local p=sliderSetValueProbe
        local observed={}
        for _,ch in ipairs(p.channels) do
            for _,entry in ipairs({{ch.init,'Init'},{ch.handler,'OnSliderChanged'},
                                   {ch.setVolume,'SetVolume'}}) do
                assert(observed[entry[1]]==nil)
                observed[entry[1]]={channel=ch,phase=entry[2]}
            end
        end
        local providerStack={}
        local function hook(event)
            if event~='call' and event~='return' then return end
            local info=debug.getinfo(2,'f')
            local fn=info and info.func
            if fn==p.provider then
                if event=='call' then
                    local _,id=debug.getlocal(2,2)
                    local _,value=debug.getlocal(2,3)
                    assert(type(id)=='number' and type(value)=='number','ADMISSION: actual provider arguments')
                    providerStack[#providerStack+1]={id,value}
                    p.allWrites[#p.allWrites+1]={id,value}
                    if p.byId[id] then
                        p.trace[#p.trace+1]={'provider_set',id,value}
                        p.channelWrites[#p.channelWrites+1]={id,value}
                    end
                else
                    local row=providerStack[#providerStack]
                    assert(row,'ADMISSION: balanced actual provider return')
                    providerStack[#providerStack]=nil
                    p.allProviderReturns[#p.allProviderReturns+1]=row
                    if p.byId[row[1]] then
                        p.trace[#p.trace+1]={'provider_return',row[1],row[2]}
                        p.channelProviderReturns[#p.channelProviderReturns+1]=row
                    end
                end
                return
            end
            if event~='call' then return end
            if fn==p.setOptionItemValue then
                local _,id=debug.getlocal(2,1)
                local _,value=debug.getlocal(2,2)
                assert(type(id)=='number' and type(value)=='number','ADMISSION: original option setter arguments')
                if p.byId[id] then p.trace[#p.trace+1]={'SetOptionItemValue',id,value} end
                return
            end
            local entry=fn and observed[fn]
            if not entry then return end
            local _,self=debug.getlocal(2,1)
            assert(self==entry.channel.control,'ADMISSION: original call receiver')
            local value
            if entry.phase=='SetVolume' then
                local name;name,value=debug.getlocal(2,2)
                assert(name and type(value)=='number','ADMISSION: original SetVolume value')
            end
            -- No widget/native query occurs inside the hook; it cannot flush queued UI work.
            p.trace[#p.trace+1]={entry.phase,entry.channel.id,value}
        end
        debug.sethook(hook,'cr')
        local ok,err=pcall(function() p.sound:Init() end)
        debug.sethook()
        assert(ok,err)
        assert(#providerStack==0)
        assert(X2Option.SetItemFloatValue==p.provider and SetOptionItemValue==p.setOptionItemValue)
        p.final={}
        for i,ch in ipairs(p.channels) do
            assert(ch.control.Init==ch.init and ch.control.SetVolume==ch.setVolume)
            assert(p.registry[ch.widgetId].OnSliderChanged==ch.handler)
            p.final[i]={ch.id,ch.control:GetValue(),GetOptionItemValue(ch.id)}
        end
    "#).expect("ADMISSION: unchanged complete Sound Init returned without replacing original callbacks");
}

#[cfg(not(test))]
pub(crate) use fake::{config, physical_pixels};

/// Display the original full Sound page. This is a smoke scene, not a visible before/after fix.
#[cfg(not(test))]
pub fn show_original_sound(ui: &UiRuntime) {
    ui.exec("@setvalue_original_sound_page", r#"
        UIParent:SetUIScale(.85)
        optionFrame:Show(true)
        optionFrame:SelectSubFrame(3)
        assert(sliderSetValueProbe.sound:IsVisible())
    "#).expect("original full Sound page at .85");
}

/// The visible example asserts actual provider calls, not audio/backend or pixel parity.
#[cfg(not(test))]
pub fn application_report(ui: &UiRuntime) -> Vec<[f64; 2]> {
    let writes: Vec<[f64; 2]> = ui.lua.load("return sliderSetValueProbe.channelWrites").eval().unwrap();
    let returned: Vec<[f64; 2]> = ui.lua.load("return sliderSetValueProbe.channelProviderReturns").eval().unwrap();
    let values: Vec<[f64; 3]> = ui.lua.load("return sliderSetValueProbe.final").eval().unwrap();
    let ids = [35., 36., 37., 39., 40.];
    let expected = ids.map(|id| [id, 1.]).to_vec();
    assert_eq!(writes, expected, "five actual original Sound provider applications");
    assert_eq!(returned, expected, "actual existing provider returns");
    assert_eq!(values, ids.map(|id| [id, 10., 1.]).to_vec());
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
    writes
}
