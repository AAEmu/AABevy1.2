//! Native named-only drawable RGBA and original chat options save.
#[path = "../examples/native_chat_options.rs"]
#[allow(dead_code)]
mod original;
use aa_ui::UiRuntime;

#[test]
fn getter_returns_one_fresh_named_only_table_with_unclamped_f32_values() {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    ui.exec(
        "@drawable_color",
        r#"
        owner=UIParent:CreateWidget('window','colorOwner','UIParent')
        color=owner:CreateColorDrawable(.25,.5,.75,.8,'background')
        assert(select('#',color:GetColor())==1)
        local first=color:GetColor();local count=0
        for k,v in pairs(first) do
            assert(k=='r' or k=='g' or k=='b' or k=='a');assert(type(v)=='number');count=count+1
        end
        assert(count==4 and #first==0 and first[1]==nil and first[4]==nil)
        assert(first.r==.25 and first.g==.5 and first.b==.75)
        assert(math.abs(first.a-.8)<.000001)
        first.r=99;first.a=0
        local second=color:GetColor();assert(second~=first and second.r==.25 and second.a>0)
        color:SetColor(-1,2,1/3,0)
        local third=color:GetColor();assert(third.r==-1 and third.g==2 and third.a==0)
        assert(math.abs(third.b-1/3)<.000001 and second.r==.25 and first.r==99)
        assert(select('#',color.GetColor(owner))==0)
    "#,
    )
    .unwrap();
}

#[test]
fn original_options_physical_ok_keeps_named_palette_and_background_color_components() {
    let mut ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    original::install(&mut ui);
    original::save(&ui);
    ui.exec("@original_saved_color",r#"
        local c=savedChatInfo.filterColors[2];assert(c.r==.25 and c.g==.5 and c.b==.75)
        assert(math.abs(c.a-.8)<.000001 and c[1]==nil and c[4]==nil)
        local bg=savedChatInfo.bgColor
        assert(math.abs(bg.r-.1)<.000001 and math.abs(bg.g-.2)<.000001 and math.abs(bg.b-.3)<.000001)
        assert(bg.a==1)
    "#).unwrap();
    assert!(ui.errors.is_empty(), "{:?}", ui.errors);
}
