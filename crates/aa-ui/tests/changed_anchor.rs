//! Native 39276dd0/39276350/395129d0: synchronous, zero-extra-argument event 32.
use aa_ui::{UiRuntime, lua};

fn run(ui: &UiRuntime, code: &str) {
    ui.exec("@changed_anchor", code).unwrap();
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
                assert(self==w and select('#',...)==0)
                table.insert(log,name)
            end)
            return w
        end
        w=widget('w');t=widget('t');c=widget('c');g=widget('g');other=widget('other')
    "#,
    );
    ui
}

#[test]
fn setters_notify_synchronously_even_without_geometry_changes_but_removal_and_cycles_do_not() {
    let ui = setup();
    run(
        &ui,
        r#"
        assert(select('#',w:AddAnchor('TOPLEFT',t,10,20))==0)
        assert(table.concat(log,',')=='w')
        local x,y=w:GetOffset();assert(x==10 and y==20)
        w:AddAnchor('TOPLEFT',t,10,20)
        assert(table.concat(log,',')=='w,w')
        w:RemoveAllAnchors();w:RemoveAllAnchors()
        assert(#log==2)
        w:AddAnchor('nonsense',t,0,0);w:AddAnchor('TOPLEFT',w,0,0)
        w:AddAnchor('TOPLEFT',{},0,0);w:AddAnchor('TOPLEFT','missingAnchorTarget',0,0)
        w:AddAnchor('TOPLEFT',t,'bad',0)
        assert(#log==2)
        log={};w:SetWidth(100);w:SetHeight(60);w:SetExtent(100,60)
        assert(table.concat(log,',')=='w,w,w')
        log={};w:SetWidth(100,false);w:SetHeight(60,0);w:SetExtent(100,0)
        assert(#log==0)
        w:SetWidth(100,'invalid');w:SetHeight(60,nil);w:SetExtent(100,60,false)
        assert(table.concat(log,',')=='w,w,w')
        w:SetWidth();w:SetHeight('60');w:SetExtent(100);w:SetExtent(false,60)
        assert(#log==3)
    "#,
    );
    lua::tick(&ui.lua, 0.);
    run(&ui, "assert(#log==3)"); // No delayed/retried anchor event from bounds::update.
}

#[test]
fn hidden_nonchildren_propagate_in_dependency_order_with_diamond_duplicates() {
    let ui = setup();
    run(
        &ui,
        r#"
        -- Registration order differs from widget creation order. No widget is shown.
        other:AddAnchor('TOPLEFT',w,0,0)
        c:AddAnchor('TOPLEFT',w,0,0)
        g:AddAnchor('TOPLEFT',other,0,0);g:AddAnchor('BOTTOMRIGHT',c,0,0)
        unanchored=widget('unanchored',w)
        log={};w:AddAnchor('TOPLEFT',t,30,40)
        assert(table.concat(log,',')=='other,g,c,w')
        local x,y=c:GetOffset();assert(x==30 and y==40)
        g:SetHandler('OnChangedAnchor',function(self)
            table.insert(log,'g'..self:GetWidth())
        end)
        log={};w:AddAnchor('TOPLEFT',t,31,40)
        assert(table.concat(log,',')=='g99,other,g100,c,w')
        log={};w:AddAnchor('TOPLEFT',g,0,0);assert(#log==0)
    "#,
    );
}

#[test]
fn replacing_a_point_keeps_native_old_target_dependency_until_remove_all() {
    let ui = setup();
    run(
        &ui,
        r#"
        w:AddAnchor('TOPLEFT',t,0,0);w:AddAnchor('TOPLEFT',other,0,0)
        log={};t:AddAnchor('TOPLEFT',c,40,0)
        assert(table.concat(log,',')=='w,t')
        local x=w:GetOffset();assert(x==0)
        -- Stale native dependency records also reject cycles.
        log={};t:AddAnchor('TOPLEFT',w,0,0);assert(#log==0)
        w:RemoveAllAnchors();w:AddAnchor('TOPLEFT',other,0,0)
        log={};t:AddAnchor('TOPLEFT',c,50,0)
        assert(table.concat(log,',')=='t')
    "#,
    );
}

#[test]
fn callbacks_reenter_and_read_final_geometry_before_the_setter_returns() {
    let ui = setup();
    run(
        &ui,
        r#"
        local depth=0
        w:SetHandler('OnChangedAnchor',function(self,...)
            assert(self==w and select('#',...)==0)
            local x,y=self:GetOffset();table.insert(log,x..':'..y)
            if depth==0 then
                depth=1;self:AddAnchor('TOPLEFT',t,99,88)
                table.insert(log,'returned')
            end
        end)
        w:AddAnchor('TOPLEFT',t,10,20)
        assert(table.concat(log,',')=='10:20,99:88,returned')
        local x,y=w:GetOffset();assert(x==99 and y==88)
    "#,
    );
}

#[test]
fn callbacks_observe_live_dependents_and_handlers_and_can_retire() {
    let ui = setup();
    run(
        &ui,
        r#"
        c:AddAnchor('TOPLEFT',w,0,0);other:AddAnchor('TOPLEFT',w,0,0)
        c:SetHandler('OnChangedAnchor',function(self)
            table.insert(log,'c');self:RemoveAllAnchors()
            w:SetHandler('OnChangedAnchor',function() table.insert(log,'new') end)
        end)
        log={};w:AddAnchor('TOPLEFT',t,5,0)
        -- The native live vector advances past the following shifted entry.
        assert(table.concat(log,',')=='c,new')
        other:SetHandler('OnChangedAnchor',function(self)
            table.insert(log,'other');self:SetHandler('OnChangedAnchor',nil)
            g:AddAnchor('TOPLEFT',w,0,0)
        end)
        log={};w:AddAnchor('TOPLEFT',t,6,0)
        assert(table.concat(log,',')=='other,g,g,new')
        w:SetHandler('OnChangedAnchor',function(self)
            self:SetHandler('OnChangedAnchor',nil);self:EnableHidingIsRemove(true)
            self:Show(true);self:Show(false)
        end)
        w:AddAnchor('TOPLEFT',t,7,0)
    "#,
    );
    lua::tick(&ui.lua, 0.);
    run(
        &ui,
        "log={};t:AddAnchor('TOPLEFT',UIParent,10,20);assert(table.concat(log,',')=='t')",
    );
}

#[test]
fn handler_errors_do_not_retry_the_anchor_notification() {
    let ui = setup();
    run(
        &ui,
        r#"
        count=0
        w:SetHandler('OnChangedAnchor',function() count=count+1;error('expected anchor fixture') end)
        w:AddAnchor('TOPLEFT',UIParent,50,60)
        assert(count==1)
    "#,
    );
    lua::tick(&ui.lua, 0.);
    run(
        &ui,
        "assert(count==1);local x,y=w:GetOffset();assert(x==50 and y==60)",
    );
}

#[test]
fn scale_propagates_owned_children_then_anchors_before_on_scale_including_noops() {
    let ui = setup();
    run(
        &ui,
        r#"
        child=widget('child',w);grandchild=widget('grandchild',child)
        child:AddAnchor('TOPLEFT',w,0,0);grandchild:AddAnchor('TOPLEFT',child,0,0)
        w:SetHandler('OnScale',function() table.insert(log,'scale') end)
        log={};w:SetScale(1)
        assert(table.concat(log,',')=='grandchild,grandchild,child,grandchild,child,w,scale')
        log={};w:SetScale(1)
        assert(table.concat(log,',')=='grandchild,grandchild,child,grandchild,child,w,scale')
        log={};w:SetAlpha(.2);assert(#log==0)
    "#,
    );
}

#[test]
fn size_control_suppresses_dependents_and_drawables_propagate_to_anchored_widgets() {
    let ui = setup();
    run(
        &ui,
        r#"
        c:AddAnchor('TOPLEFT',w,0,0)
        log={};w:SetWidth(130,false);assert(#log==0)
        w:SetWidth(130);assert(table.concat(log,',')=='c,w')
        art=w:CreateColorDrawable(1,1,1,1,'artwork')
        c:RemoveAllAnchors();c:AddAnchor('TOPLEFT',art,0,0)
        log={};art:AddAnchor('TOPLEFT',UIParent,30,40)
        assert(table.concat(log,',')=='c')
        log={};art:RemoveAllAnchors();assert(#log==0)
        art:SetExtent(40,60);assert(table.concat(log,',')=='c')
    "#,
    );
}

#[test]
fn retiring_an_anchor_target_notifies_live_dependents_after_detaching_it() {
    let ui = setup();
    run(
        &ui,
        r#"
        w:AddAnchor('TOPLEFT',t,30,40);c:AddAnchor('TOPLEFT',w,10,20)
        w:EnableHidingIsRemove(true);w:Show(true);log={};w:Show(false)
    "#,
    );
    lua::tick(&ui.lua, 0.);
    run(
        &ui,
        r#"
        assert(table.concat(log,',')=='c')
        local x,y=c:GetOffset();assert(x==40 and y==60)
        log={};t:AddAnchor('TOPLEFT',UIParent,100,200)
        assert(table.concat(log,',')=='t')
        x,y=c:GetOffset();assert(x==40 and y==60)
    "#,
    );
}

#[test]
fn silent_nested_size_setter_updates_bounds_without_delivering_another_handler() {
    let ui = setup();
    run(
        &ui,
        r#"
        c:AddAnchor('TOPLEFT',w,0,0);c:AddAnchor('BOTTOMRIGHT',w,0,0)
        w:SetHandler('OnChangedAnchor',function(self)
            table.insert(log,'w')
            self:SetWidth(150,false)
            assert(self:GetWidth()==150 and c:GetWidth()==150)
        end)
        log={};w:AddAnchor('TOPLEFT',t,10,20)
        assert(table.concat(log,',')=='c,w')
        assert(w:GetWidth()==150 and c:GetWidth()==150)
    "#,
    );
}

#[test]
fn captured_parent_motion_propagates_the_existing_anchor_without_resetting_readiness() {
    let ui = setup();
    run(
        &ui,
        r#"
        other:AddAnchor('TOPLEFT',w,0,0);c:AddAnchor('TOPLEFT',w,0,0)
        g:AddAnchor('TOPLEFT',other,0,0);g:AddAnchor('BOTTOMRIGHT',c,0,0)
        w:AddAnchor('TOPLEFT',UIParent,30,40)
        w:StartMoving()
        g:SetHandler('OnChangedAnchor',function(self) table.insert(log,'g'..self:GetWidth()) end)
        log={}
    "#,
    );
    aa_ui::input::update(
        &ui.lua,
        &mut aa_ui::input::MouseState::default(),
        Some((1., 0.)),
        false,
        false,
        false,
        0.,
    );
    run(
        &ui,
        r#"
        assert(table.concat(log,',')=='g99,other,g100,c,w')
        local x,y=w:GetOffset();assert(x==31 and y==40)
        w:StopMovingOrSizing()
        reanchored=false
        w:SetHandler('OnChangedAnchor',function(self)
            if not reanchored then
                reanchored=true;self:AddAnchor('TOPLEFT',UIParent,99,88)
            end
        end)
        w:StartMoving()
    "#,
    );
    aa_ui::input::update(
        &ui.lua,
        &mut aa_ui::input::MouseState::default(),
        Some((2., 0.)),
        false,
        false,
        false,
        0.,
    );
    run(
        &ui,
        "local x,y=w:GetOffset();assert(x==100 and y==88);w:StopMovingOrSizing()",
    );
}

#[test]
fn reentrant_scale_notifies_its_source_while_an_anchor_target_is_still_pending() {
    let ui = setup();
    run(
        &ui,
        r#"
        other:AddAnchor('TOPLEFT',w,0,0);c:AddAnchor('TOPLEFT',w,0,0)
        t:AddAnchor('TOPLEFT',c,0,0)
        other:SetHandler('OnChangedAnchor',function()
            table.insert(log,'other')
            g:AddAnchor('TOPLEFT',t,0,0)
            assert(table.concat(log,',')=='other')
            g:SetScale(1)
        end)
        g:SetHandler('OnScale',function() table.insert(log,'scale') end)
        log={};w:AddAnchor('TOPLEFT',UIParent,30,40)
        assert(table.concat(log,',')=='other,g,scale,g,t,c,w')
    "#,
    );
}
