//! Native event33: size setter -> all event32 -> source event33 -> dependent event33.
use aa_ui::{UiRuntime, input, lua};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@bound_changed", code).expect(code);
}

fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1920., 1080.).unwrap();
    run(
        &ui,
        r#"
        log={}
        function widget(name,parent)
            local w=UIParent:CreateWidget('window',name,parent or 'UIParent')
            w:SetExtent(100,60)
            w:SetHandler('OnChangedAnchor',function(self,...) 
                assert(self==w and select('#',...)==0);table.insert(log,name..'32')
            end)
            w:SetHandler('OnBoundChanged',function(self,...)
                assert(self==w and select('#',...)==0);table.insert(log,name..'33')
            end)
            return w
        end
        w=widget('w');t=widget('t');c=widget('c');g=widget('g');other=widget('other')
        function expect(value) assert(table.concat(log,',')==value,table.concat(log,','));log={} end
    "#,
    );
    ui
}

#[test]
fn identical_size_setters_notify_synchronously_in_two_distinct_orders() {
    let ui = setup();
    run(
        &ui,
        r#"
        c:AddAnchor('TOPLEFT',w,0,0);g:AddAnchor('TOPLEFT',c,0,0)
        other:AddAnchor('TOPLEFT',w,0,0);log={}
        w:SetWidth(100);expect('g32,c32,other32,w32,w33,c33,g33,other33')
        w:SetHeight(60);expect('g32,c32,other32,w32,w33,c33,g33,other33')
        w:SetExtent(100,60);expect('g32,c32,other32,w32,w33,c33,g33,other33')
        w:SetWidth(100,false);w:SetHeight(60,0);w:SetExtent(100,0);expect('')
        w:SetExtent(100,60,false);expect('g32,c32,other32,w32,w33,c33,g33,other33')
        w:SetWidth(100,'bad');expect('g32,c32,other32,w32,w33,c33,g33,other33')
        w:SetHeight(60,nil);expect('g32,c32,other32,w32,w33,c33,g33,other33')
        w:SetWidth();w:SetHeight('60');w:SetExtent(100);expect('')
    "#,
    );
    lua::tick(&ui.lua, 0.);
    run(&ui, "expect('')");
}

#[test]
fn registration_replacement_and_first_tick_never_invent_a_bound_notification() {
    let ui = setup();
    lua::tick(&ui.lua, 0.);
    run(
        &ui,
        r#"
        expect('')
        w:SetHandler('OnBoundChanged',function() table.insert(log,'replacement') end)
        expect('');w:SetWidth(100);expect('w32,replacement')
        w:ReleaseHandler('OnBoundChanged');w:SetWidth(100);expect('w32')
        w:SetHandler('OnBoundChanged',function() table.insert(log,'new') end);expect('')
    "#,
    );
    lua::tick(&ui.lua, 0.);
    run(&ui, "expect('')");
}

#[test]
fn hidden_diamond_repeats_bounds_even_when_child_extent_is_unchanged() {
    let ui = setup();
    run(
        &ui,
        r#"
        other:AddAnchor('TOPLEFT',w,0,0);c:AddAnchor('TOPLEFT',w,0,0)
        g:AddAnchor('TOPLEFT',other,0,0);g:AddAnchor('BOTTOMRIGHT',c,0,0);log={}
        w:SetWidth(101)
        expect('g32,other32,g32,c32,w32,w33,other33,g33,c33,g33')
        assert(c:GetWidth()==100 and not c:IsVisible())
        unanchored=widget('unanchored',w);log={};w:SetWidth(102)
        expect('g32,other32,g32,c32,w32,w33,other33,g33,c33,g33')
        -- A replaced point retains its native old-target record for both events.
        c:AddAnchor('TOPLEFT',t,0,0);log={};w:SetWidth(103)
        expect('g32,other32,g32,c32,w32,w33,other33,g33,c33,g33')
        c:RemoveAllAnchors();log={};w:SetWidth(104)
        expect('g32,other32,w32,w33,other33,g33')
    "#,
    );
}

#[test]
fn anchored_parent_resize_resolves_geometry_before_bound_delivery() {
    let ui = setup();
    run(
        &ui,
        r#"
        c:AddAnchor('TOPLEFT',w,0,0);c:AddAnchor('BOTTOMRIGHT',w,0,0)
        w:SetHandler('OnBoundChanged',function(self)
            assert(self:GetWidth()==150 and c:GetWidth()==150);table.insert(log,'w33')
        end)
        c:SetHandler('OnBoundChanged',function(self)
            assert(self:GetWidth()==150);table.insert(log,'c33')
        end)
        log={};w:SetWidth(150);expect('c32,w32,w33,c33')
        w:SetHandler('OnChangedAnchor',function(self)
            table.insert(log,'w32');self:SetWidth(150,false)
        end)
        w:SetWidth(120);expect('c32,w32,w33,c33')
    "#,
    );
}

#[test]
fn bound_reentry_finishes_before_the_outer_dependent_walk() {
    let ui = setup();
    run(
        &ui,
        r#"
        c:AddAnchor('TOPLEFT',w,0,0);log={};local nested=false
        w:SetHandler('OnBoundChanged',function(self)
            table.insert(log,'w33:'..self:GetWidth())
            if not nested then
                nested=true;self:SetWidth(150);table.insert(log,'returned')
            end
        end)
        w:SetWidth(120)
        expect('c32,w32,w33:120,c32,w32,w33:150,c33,returned,c33')
    "#,
    );
}

#[test]
fn bound_notification_does_not_wait_for_anchor_readiness_during_reentry() {
    let ui = setup();
    run(
        &ui,
        r#"
        other:AddAnchor('TOPLEFT',w,0,0);c:AddAnchor('TOPLEFT',w,0,0)
        t:AddAnchor('TOPLEFT',c,0,0)
        other:SetHandler('OnChangedAnchor',function()
            table.insert(log,'other32');g:AddAnchor('TOPLEFT',t,0,0)
            g:SetWidth(110)
        end)
        log={};w:AddAnchor('TOPLEFT',UIParent,30,40)
        expect('other32,g33,g32,t32,c32,w32')
    "#,
    );
}

#[test]
fn callbacks_observe_live_handlers_and_source_mutations_before_traversal() {
    let ui = setup();
    run(
        &ui,
        r#"
        c:AddAnchor('TOPLEFT',w,0,0);log={}
        w:SetHandler('OnChangedAnchor',function(self)
            table.insert(log,'w32');self:ReleaseHandler('OnBoundChanged')
        end)
        w:SetWidth(120);expect('c32,w32,c33')
        w:ReleaseHandler('OnChangedAnchor')
        w:SetHandler('OnBoundChanged',function()
            table.insert(log,'w33');c:ReleaseHandler('OnBoundChanged')
            other:AddAnchor('TOPLEFT',w,0,0)
        end)
        w:SetWidth(130);expect('c32,w33,other32,other33')
        w:SetHandler('OnBoundChanged',function()
            table.insert(log,'w33');other:RemoveAllAnchors()
            c:SetHandler('OnBoundChanged',function() table.insert(log,'new') end)
        end)
        w:SetWidth(140);expect('c32,other32,w33,new')
    "#,
    );
}

#[test]
fn handler_failure_continues_dependents_without_delayed_retry() {
    let ui = setup();
    run(
        &ui,
        r#"
        c:AddAnchor('TOPLEFT',w,0,0);log={}
        w:SetHandler('OnBoundChanged',function()
            table.insert(log,'error');error('expected bound fixture failure')
        end)
        w:SetWidth(100);expect('c32,w32,error,c33')
    "#,
    );
    lua::tick(&ui.lua, 0.);
    run(&ui, "expect('')");
}

#[test]
fn anchor_remove_scale_drawable_and_captured_movement_do_not_synthesize_bounds() {
    let ui = setup();
    run(
        &ui,
        r#"
        c:AddAnchor('TOPLEFT',w,0,0);log={}
        w:AddAnchor('TOPLEFT',t,30,40);expect('c32,w32')
        w:AddAnchor('TOPLEFT',t,30,40);expect('c32,w32')
        w:RemoveAllAnchors();w:RemoveAllAnchors();expect('')
        w:SetHandler('OnScale',function() table.insert(log,'scale') end)
        w:SetScale(1);expect('c32,w32,scale')
        w:SetScale(2);expect('c32,w32,scale')
        art=w:CreateColorDrawable(1,1,1,1,'artwork')
        c:RemoveAllAnchors();c:AddAnchor('TOPLEFT',art,0,0);log={}
        art:SetExtent(30,40);expect('c32')
        art:AddAnchor('TOPLEFT',w,0,0);expect('c32')
        w:SetWidth(100);expect('c32,w32,w33')
        c:RemoveAllAnchors();c:AddAnchor('TOPLEFT',w,0,0)
        w:SetScale(1);w:AddAnchor('TOPLEFT',UIParent,30,40);log={}
        w:StartMoving();expect('c32,w32')
    "#,
    );
    input::update(
        &ui.lua,
        &mut input::MouseState::default(),
        Some((10., 20.)),
        false,
        false,
        false,
        0.,
    );
    run(&ui, "expect('c32,w32');w:StopMovingOrSizing();expect('')");
    lua::tick(&ui.lua, 0.);
    run(&ui, "expect('')");
}

#[test]
fn retirement_during_either_phase_stops_dispatch_to_retired_objects() {
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
        w=UIParent:CreateWidget('chatwindow','retireAnchor','UIParent');w:SetExtent(100,60)
        c:AddAnchor('TOPLEFT',w,0,0);log={}
        w:SetHandler('OnBoundChanged',function() error('retired source bound') end)
        w:SetHandler('OnChangedAnchor',function(self)
            table.insert(log,'retire');resetChats()
        end)
        w:SetWidth(100);expect('c32,retire,c32')
        other=UIParent:CreateWidget('chatwindow','retireBound','UIParent');other:SetExtent(100,60)
        other:SetHandler('OnChangedAnchor',function() table.insert(log,'other32') end)
        g:AddAnchor('TOPLEFT',other,0,0);log={}
        other:SetHandler('OnBoundChanged',function(self)
            table.insert(log,'retireBound');resetChats()
        end)
        other:SetWidth(100);expect('g32,other32,retireBound,g32')
    "#,
    );
    lua::tick(&ui.lua, 0.);
    run(&ui, "expect('')");
}
