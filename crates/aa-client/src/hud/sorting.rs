//! Native local bag/bank order; network inventory and physical request slots remain authoritative.
use super::api::Shared;
use mlua::{Lua, Table};

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let state = bridge.clone();
    let hud: Table = lua.globals().get("HUD")?;
    hud.set("inventory_owner", lua.create_function(move |_, ()| Ok(state.borrow().character_id))?)?;
    lua.load(include_str!("sorting.lua")).set_name("@sorting.lua").exec()
}

#[allow(dead_code)]
pub(super) mod fixture;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_default_manual_order_empty_updates_filtered_u64_and_reload_identity() -> mlua::Result<()> {
        let lua = Lua::new();
        lua.load(
            r#"
          owner=47; selected=1; bankSelected=1; saved={}; events={}
          HUD={bag_slots=8,bank_slots=8,bag={},bank={},inventory_owner=function() return owner end}
          X2Bag={CurrentTabIdx=function() return selected end,TabInfo=function() return 'test',1,groups end,SlotByIdx=function() end}
          X2Bank={CurrentTabIdx=function() return bankSelected end,TabInfo=X2Bag.TabInfo,SlotByIdx=function() end}
          groups={1}
          NET={inventory_order=function(o,c) return saved[o..':'..c] end,
            save_inventory_order=function(o,c,v) saved[o..':'..c]=v end}
          FAKE={fire_now=function(name,tab) events[#events+1]={name,tab} end}
          local function item(uid,name,cat,grade) return {uid=uid,name=name,category_order=cat,grade=grade,group=1} end
          HUD.bag[6]=item('9007199254740993','alpha',0,1)
          HUD.bag[2]=item('9007199254740994','beta',1,5)
          HUD.bag[4]=item('8','Alpha',1,5)
          HUD.bag[5]=item('2','Alpha',1,5)
          HUD.bag[7]=item('12','A',1,1)
          HUD.bank[8]=item('44','bank',0,1)
        "#,
        )
        .exec()?;
        lua.load(include_str!("sorting.lua")).exec()?;
        lua.load(
            r#"
          assert(X2Bag:SlotByIdx(2)==2 and HUD.inventory_dest(2,1)==1)
          X2Bag:ManualSort()
          local want={6,4,5,2,7}
          for i,v in ipairs(want) do assert(X2Bag:SlotByIdx(i)==v) end
          assert(X2Bag:SlotByIdx(6)==nil and HUD.inventory_dest(2,6)==1)
          assert(events[#events][1]=='BAG_TAB_SORTED' and events[#events][2]==1)
          assert(HUD.bag[6].uid=='9007199254740993' and HUD.bag[1]==nil)
          X2Bag:Sort(); assert(X2Bag:SlotByIdx(1)==6)
          assert(X2Bank:SlotByIdx(8)==8)
          X2Bank:ManualSort(); assert(X2Bank:SlotByIdx(1)==8)
          selected=2
          local want={5,4,7,6,2}
          for i,v in ipairs(want) do assert(X2Bag:SlotByIdx(i)==v) end
          X2Bag:ManualSort(); assert(X2Bag:SlotByIdx(1)==5)
          assert(events[#events][2]==2 and saved['47:2'][1].slot==6)
          selected=1
          -- A confirmed removal orders only existing empty positions, not occupied entries.
          local next={}; for i,v in pairs(HUD.bag) do if i~=4 then next[i]=v end end
          HUD.bag=next
          assert(X2Bag:SlotByIdx(1)==6 and HUD.inventory_dest(2,2)==1)
          assert(X2Bag:SlotByIdx(3)==5 and X2Bag:SlotByIdx(4)==2 and X2Bag:SlotByIdx(5)==7)
          assert(HUD.inventory_dest(2,6)==3 and HUD.inventory_dest(2,7)==4)
          -- A new item occupies that physical slot without automatic comparator sorting.
          next={}; for i,v in pairs(HUD.bag) do next[i]=v end
          next[1]={uid='99',name='AAA',category_order=-1,grade=9,group=1}; HUD.bag=next
          assert(X2Bag:SlotByIdx(1)==6 and X2Bag:SlotByIdx(2)==1)
          -- Ordinary item replacement also preserves occupied positions.
          next={}; for i,v in pairs(HUD.bag) do next[i]=v end
          next[6]={uid='100',name='replacement',category_order=5,grade=0,group=1}; HUD.bag=next
          assert(X2Bag:SlotByIdx(1)==6)
        "#,
        )
        .exec()?;
        // A new manager validates current occupied slot/UID pairs against the persisted record.
        lua.load(include_str!("sorting.lua")).exec()?;
        lua.load(
            r#"
          assert(X2Bag:SlotByIdx(1)==1 and X2Bag:SlotByIdx(2)==2, 'stale saved UID must restore identity')
          X2Bag:ManualSort(); assert(X2Bag:SlotByIdx(1)==1)
        "#,
        )
        .exec()?;
        lua.load(include_str!("sorting.lua")).exec()?;
        lua.load(
            r#"
          assert(X2Bag:SlotByIdx(1)==1 and X2Bag:SlotByIdx(2)==5, 'matching saved UIDs restore the view')
          owner=48; assert(X2Bag:SlotByIdx(2)==2 and X2Bank:SlotByIdx(8)==8)
          assert(HUD.inventory_dest(2,0)==nil and HUD.inventory_dest(2,9)==nil)
        "#,
        )
        .exec()
    }

    #[test]
    #[ignore = "loads the original world-stage assets; run in an isolated fixture process"]
    fn original_sort_button_drag_and_empty_destination_use_physical_slots() -> Result<(), Box<dyn std::error::Error>> {
        use aa_net::state::ActionBar;
        use aa_net::{Command, NetCommand};
        use aa_ui::UiRuntime;
        use bevy::prelude::*;
        let (rt, bridge, path, inventory) = fixture::runtime(1920.0, 1080.0)?;
        super::super::expansion::fixture::click(&rt, "newbag.sortBtn")?;
        rt.lua.load("assert(X2Bag:SlotByIdx(1)==5 and HUD.inventory_dest(2,2)==1)").exec()?;
        super::super::expansion::fixture::click(&rt, "newbag.slots.btns[1]")?;
        rt.lua
            .load("local h=X2Cursor:GetHeld(); assert(h.kind=='bag' and h.slot==5 and h.uid=='9007199254740993')")
            .exec()?;
        super::super::expansion::fixture::click(&rt, "newbag.slots.btns[2]")?;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_non_send(rt)
            .insert_non_send(super::super::api::HudBridge(bridge.clone()))
            .insert_resource(inventory)
            .insert_resource(ActionBar::default())
            .add_message::<NetCommand>()
            .add_systems(Update, super::super::drag::apply_requests);
        app.update();
        let got = app.world_mut().resource_mut::<Messages<NetCommand>>().drain().map(|v| v.0).collect::<Vec<_>>();
        assert!(matches!(got.as_slice(),[Command::SwapItems(mv)] if mv.from_slot==4 && mv.to_slot==0 && mv.from_item_id==9_007_199_254_740_993));
        let received = app.world().resource::<aa_net::state::Inventory>();
        assert!(received.bag[0].is_none());
        assert_eq!(received.bag[4].as_ref().unwrap().id, 9_007_199_254_740_993);
        assert_eq!(bridge.borrow().panels.bag[4].as_ref().unwrap().uid, 9_007_199_254_740_993);
        fixture::show_bank(app.world().non_send::<UiRuntime>())?;
        super::super::expansion::fixture::click(app.world().non_send::<UiRuntime>(), "GetBankInventory().sortBtn")?;
        app.world()
            .non_send::<UiRuntime>()
            .lua
            .load("assert(X2Bank:SlotByIdx(1)==8 and HUD.inventory_dest(3,2)==1)")
            .exec()?;
        // Both cross-container paths and the bank's own empty cells use the mapped physical slot.
        for (from, to, containers, slots, id) in [
            (
                "GetBankInventory().slots.btns[1]",
                "GetBankInventory().slots.btns[2]",
                (3, 3),
                (7, 0),
                9_007_199_254_740_995,
            ),
            (
                "GetBankInventory().slots.btns[1]",
                "newbag.slots.btns[2]",
                (3, 2),
                (7, 0),
                9_007_199_254_740_995,
            ),
            (
                "newbag.slots.btns[1]",
                "GetBankInventory().slots.btns[2]",
                (2, 3),
                (4, 0),
                9_007_199_254_740_993,
            ),
        ] {
            super::super::expansion::fixture::click(app.world().non_send::<UiRuntime>(), from)?;
            super::super::expansion::fixture::click(app.world().non_send::<UiRuntime>(), to)?;
            app.update();
            let got = app.world_mut().resource_mut::<Messages<NetCommand>>().drain().map(|v| v.0).collect::<Vec<_>>();
            assert!(
                matches!(got.as_slice(),[Command::SwapItems(mv)] if (mv.from_container,mv.to_container)==containers && (mv.from_slot,mv.to_slot)==slots && mv.from_item_id==id),
                "{from} -> {to}: {got:?}"
            );
        }
        assert_eq!(
            app.world().resource::<aa_net::state::Inventory>().bank[7].as_ref().unwrap().id,
            9_007_199_254_740_995
        );
        assert!(app.world().non_send::<UiRuntime>().errors.is_empty());
        std::fs::remove_file(path)?;
        Ok(())
    }
}
