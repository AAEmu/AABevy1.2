//! Native pixel overflow queries and unchanged pet/tooltip recovery helpers.

#[path = "../examples/fake/mod.rs"]
#[allow(dead_code)]
mod fake;
#[path = "../examples/fake/screen_bounds.rs"]
mod screen_bounds;

use aa_ui::{UiRuntime, lua};

fn run(ui: &UiRuntime, source: &str) {
    ui.exec("@screen_bounds_test", source).unwrap();
}

#[test]
fn effective_pixel_bounds_prioritize_near_edges_and_do_not_mutate_geometry() {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    run(
        &ui,
        r#"
        window=UIParent:CreateWidget('window','screenBounds','UIParent')
        window:SetExtent(100,40)
        local cases={
            {0,0,false,0,0}, {1820,1040,false,1820,1040},
            {-10,10,true,0,10}, {10,-10,true,10,0},
            {1821,10,true,1820,10}, {10,1041,true,10,1040},
            {-10,1041,true,0,1040}, {1821,-10,true,1820,0}}
        for _,c in ipairs(cases) do
            window:RemoveAllAnchors();window:AddAnchor('TOPLEFT',UIParent,c[1],c[2])
            assert(window:CheckOutOfScreen()==c[3])
            local x,y=window:CorrectOffsetByScreen();assert(x==c[4] and y==c[5])
            x,y=window:GetOffset();assert(x==c[1] and y==c[2])
        end
        window:SetExtent(2100,1200)
        window:RemoveAllAnchors();window:AddAnchor('TOPLEFT',UIParent,10,10)
        local x,y=window:CorrectOffsetByScreen();assert(x==-180 and y==-120)
        window:RemoveAllAnchors();window:AddAnchor('TOPLEFT',UIParent,-10,-10)
        x,y=window:CorrectOffsetByScreen();assert(x==0 and y==0)
        window:SetExtent(200,100);window:SetScale(0.5)
        window:RemoveAllAnchors();window:AddAnchor('TOPLEFT',UIParent,1760,940)
        assert(not window:CheckOutOfScreen())
        window:RemoveAllAnchors();window:AddAnchor('TOPLEFT',UIParent,1840,1040)
        x,y=window:CorrectOffsetByScreen();assert(x==1820 and y==1030)
    "#,
    );
}

#[test]
fn removing_anchors_retains_resolved_origin_extent_and_detaches_dependencies() {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    run(
        &ui,
        r#"
        owner=UIParent:CreateWidget('window','boundsOwner','UIParent')
        owner:SetExtent(400,300);owner:AddAnchor('TOPLEFT',UIParent,100,200)
        child=owner:CreateChildWidget('emptywidget','child',0,true)
        child:AddAnchor('TOPLEFT',owner,20,30);child:AddAnchor('BOTTOMRIGHT',owner,-20,-30)
        child:RemoveAllAnchors()
        local x,y=child:GetOffset();local w,h=child:GetExtent()
        assert(x==120 and y==230 and w==360 and h==240)
        owner:RemoveAllAnchors();owner:AddAnchor('TOPLEFT',UIParent,400,500)
        x,y=child:GetOffset();assert(x==120 and y==230)
        child:SetExtent(100,40);w,h=child:GetExtent();assert(w==100 and h==40)
        child:AddAnchor('TOPLEFT',owner,10,15)
        x,y=child:GetOffset();assert(x==410 and y==515)
    "#,
    );
}

#[test]
fn original_pet_and_tooltip_helpers_recover_overflow_with_global_ui_scale() {
    let mut ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    screen_bounds::install(&mut ui);
    run(
        &ui,
        r#"
        local x,y=petBar:GetOffset();assert(x==1670 and y==975)
        UIParent:SetUIScale(0.8)
    "#,
    );
    // Same physical framebuffer, as the runtime's resolution/UI-scale update does.
    lua::ui(&ui.lua).tree.set_screen(2400., 1350.);
    run(
        &ui,
        r#"
        petBar:RemoveAllAnchors();petBar:AddAnchor('TOPLEFT',UIParent,2350,1300)
        assert(petBar:CheckOutOfScreen())
        local x,y=petBar:CorrectOffsetByScreen()
        assert(math.abs(x-1720)<0.01 and math.abs(y-996)<0.01)
        petBar:OnScale()
        assert(not petBar:CheckOutOfScreen())
        x,y=petBar:GetEffectiveOffset()
        assert(math.abs(x-1720)<0.01 and math.abs(y-996)<0.01)
        overflowTarget:RemoveAllAnchors();overflowTarget:AddAnchor('TOPLEFT',UIParent,2500,20)
        SetTooltip('Recovered original tooltip',overflowTarget)
        assert(not tooltip.window:CheckOutOfScreen())
    "#,
    );
}
