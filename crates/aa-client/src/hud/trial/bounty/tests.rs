use super::*;
use aa_net::BountyRow;
use std::{cell::RefCell, rc::Rc};

fn setup() -> Result<(UiRuntime, Shared), Box<dyn std::error::Error>> {
    let mut rt = UiRuntime::new("en_us", 1920., 1080.)?;
    rt.load_ui_texts(std::path::Path::new(aa_anim::db::DEFAULT_DB))?;
    let state = Rc::new(RefCell::new(HudData::default()));
    crate::hud::api::install(&mut rt, &state)?;
    super::super::fixture::selected(Some(&rt), &state);
    Ok((rt, state))
}
fn row(id: u32, amount: u64) -> BountyRow {
    BountyRow {
        player_id: id,
        name: b"Wanted\0ignored".to_vec(),
        level: 55,
        zone_id: -1,
        x: i64::MIN,
        y: i64::MAX,
        z_bits: f32::NAN.to_bits(),
        money: amount,
        crime_points: -1,
        raw_is_live: 255,
        race: 1,
        faction_id: 4,
        arrest_count: 11,
        accept_guilty_count: 12,
        accept_trial_count: 13,
        first_verdict_count: 14,
        second_verdict_count: 15,
        seconds: 16,
    }
}
fn event(rt: Option<&UiRuntime>, state: &Shared, event: Event) {
    super::super::dispatch_at(rt, state, &event, Some(1_700_000_000));
}
fn send(rt: Option<&UiRuntime>, state: &Shared, receipt: BountyEvent) {
    event(rt, state, Event::Bounty(receipt));
}
fn list(rt: Option<&UiRuntime>, state: &Shared, total: u32, rows: Vec<BountyRow>) {
    send(
        rt,
        state,
        BountyEvent::List {
            total,
            declared_count: rows.len() as u8,
            rows,
        },
    );
}
fn commands(state: &Shared) -> Vec<(u32, BountyCommand)> {
    super::super::flush(&mut state.borrow_mut())
        .into_iter()
        .map(|command| match command {
            aa_net::Command::Bounty { character_id, command } => (character_id, command),
            other => panic!("unexpected command {other:?}"),
        })
        .collect()
}

#[test]
fn original_getter_numeric_types_order_and_columns_match_the_executed_native_rows() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    list(Some(&rt), &state, 2, vec![row(u32::MAX, 0x8000_0000), row(1, 0x1_0000_0001)]);
    rt.lua
        .load(
            r#"
        assert(select('#',X2Trial:GetBountyData())==2)
        local rows,count=X2Trial:GetBountyData();assert(count==2 and #rows==2)
        assert(#rows[1]==15 and #rows[2]==15 and type(rows[1][1])=='number')
        assert(rows[1][1]==1 and rows[2][1]==4294967296)
        assert(rows[1][2]=='Wanted' and rows[1][3]==55 and rows[1][4]==-1)
        assert(rows[1][5]==1 and rows[2][5]==-2147483648 and rows[1][6]==-1)
        assert(rows[1][7]=='' and rows[1][8]=='Nuian' and rows[1][9]=='')
        for i=10,15 do assert(rows[1][i]==i+1) end
    "#,
        )
        .exec()?;
    assert_eq!(state.borrow().trial.bounty.received.by_id[&1].money, 0x1_0000_0001);
    assert!(commands(&state).is_empty());
    Ok(())
}

#[test]
fn queued_bulletin_callbacks_observe_each_native_complete_map_before_later_mutation() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    rt.lua
        .load(
            r#"
        observed={}
        local w=UIParent:CreateWidget('emptywidget','bountyListener','UIParent')
        w:RegisterEvent('TOGGLE_BOUNTY_BULLETIN')
        w:SetHandler('OnEvent',function()
          local rows,n=X2Trial:GetBountyData();observed[#observed+1]={count=n,money=rows[1] and rows[1][5]}
        end)
    "#,
        )
        .exec()?;
    list(None, &state, 2, vec![row(1, 11)]);
    list(None, &state, 2, vec![row(2, 22)]);
    list(None, &state, 2, vec![row(1, 33)]);
    list(None, &state, 2, vec![]);
    rt.lua.load("assert(#observed==0)").exec()?;
    super::super::replay(&rt, &state);
    rt.lua
        .load(
            r#"
        assert(#observed==3 and observed[1].count==2 and observed[1].money==11)
        assert(observed[2].money==33 and observed[3].money==33)
        local rows=X2Trial:GetBountyData();assert(rows[1][5]==33)
    "#,
        )
        .exec()?;
    assert!(state.borrow().trial.bounty.callback.is_none());
    Ok(())
}

#[test]
fn typed_weak_lua_controls_keep_raw_names_numeric_cvtt_and_stale_session_refusal() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    rt.lua
        .load(
            r#"
        assert(select('#',X2Trial:RequestSetBountyMoney('A\000tail'))==0)
        assert(select('#',X2Trial:SendBountyUpdate(-1,'B',-1.9))==0)
        X2Trial:SendBountyUpdate(0/0,string.char(255),0/0)
        X2Trial:RequestSetBountyMoney(1);X2Trial:RequestSetBountyMoney(string.rep('X',129))
        X2Trial:SendBountyUpdate('1','C',2);X2Trial:SendBountyUpdate(1,'C','2')
    "#,
        )
        .exec()?;
    assert_eq!(
        commands(&state),
        [
            (47, BountyCommand::RequestSetMoney { name: b"A".to_vec() }),
            (
                47,
                BountyCommand::Update {
                    kind: u32::MAX,
                    name: b"B".to_vec(),
                    money: -1
                }
            ),
            (
                47,
                BountyCommand::Update {
                    kind: 0x8000_0000,
                    name: vec![255],
                    money: i32::MIN
                }
            )
        ]
    );
    assert!(state.borrow().trial.bounty.received.by_id.is_empty());
    rt.lua.load("X2Trial:SendBountyUpdate(1,'Old',99)").exec()?;
    event(Some(&rt), &state, Event::Disconnected { message: "fixture".into() });
    super::super::fixture::selected(Some(&rt), &state);
    assert!(commands(&state).is_empty());
    Ok(())
}

#[test]
fn actor_receipts_capture_actual_name_local_flag_and_duplicates_before_removal() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup()?;
    rt.lua
        .load(
            r#"
        observed={}
        local w=UIParent:CreateWidget('emptywidget','bountyActorListener','UIParent')
        w:RegisterEvent('SET_BOUNTY_DONE');w:RegisterEvent('BOUNTY_PAID');w:RegisterEvent('SET_BOUNTY')
        w:SetHandler('OnEvent',function(self,event,...) observed[#observed+1]={event=event,args={...}} end)
    "#,
        )
        .exec()?;
    send(None, &state, BountyEvent::UnitMoney { obj_id: 999, money: 3 });
    send(
        None,
        &state,
        BountyEvent::UnitMoney {
            obj_id: 100,
            money: 0x1_0000_0001,
        },
    );
    event(
        None,
        &state,
        Event::UnitNameChanged {
            obj_id: 100,
            name: "New name\0tail".into(),
        },
    );
    send(None, &state, BountyEvent::UnitMoney { obj_id: 100, money: -1 });
    send(None, &state, BountyEvent::UnitMoney { obj_id: 100, money: -1 });
    event(None, &state, Event::UnitsRemoved(vec![100]));
    send(None, &state, BountyEvent::UnitMoney { obj_id: 100, money: 5 });
    send(
        None,
        &state,
        BountyEvent::SetDone {
            kind: 50,
            name: b"Wanted\0tail".to_vec(),
            money: 10,
            error: 0,
        },
    );
    send(
        None,
        &state,
        BountyEvent::Paid {
            hunter: b"Hunter\0tail".to_vec(),
            arrested: b"Wanted".to_vec(),
            money: 11,
        },
    );
    send(
        None,
        &state,
        BountyEvent::SetPermitted {
            kind: 50,
            name: b"Wanted".to_vec(),
            money: 12,
            raw_allowed: 1,
        },
    );
    super::super::replay(&rt, &state);
    rt.lua
        .load(
            r#"
        assert(#observed==6)
        local a=observed[1].args;assert(a[1]==100 and a[2]=='Expansion fixture' and a[3]==1 and a[4]==1)
        for i=2,3 do a=observed[i].args;assert(a[2]=='New name' and a[3]==-1 and a[4]==1) end
        a=observed[4].args;assert(a[1]==50 and a[2]=='Wanted' and a[3]==10 and a[4]==1)
        assert(observed[5].event=='BOUNTY_PAID' and observed[5].args[1]=='Hunter' and observed[5].args[3]==11)
        assert(observed[6].event=='SET_BOUNTY' and #observed[6].args==3)
    "#,
        )
        .exec()?;
    assert!(commands(&state).is_empty());
    Ok(())
}

#[test]
fn original_empty_bulletin_and_standalone_setup_controls_queue_only_the_captured_request() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state, path) = crate::hud::breath::fixture::runtime(1920., 1080.)?;
    super::super::fixture::selected(Some(&rt), &state);
    rt.lua.load("ADDON:ShowContent(UIC_BAG,false)").exec()?;
    // The lane's actual empty bulletin is supported by the original consumer.
    list(Some(&rt), &state, 0, vec![]);
    rt.lua
        .load("assert(userTrial.bountyBulletWnd:IsVisible());userTrial.bountyBulletWnd:Show(false)")
        .exec()?;
    // An incomplete received chunk does not open the known-broken nonempty
    // original bulletin (bounty_bullet.alb:115 reads undefined global listCtrl).
    // Keep that original-script boundary explicit; do not install fake globals.
    list(Some(&rt), &state, 2, vec![row(50, 12345)]);
    // Exercise the actual original setup function; the absent native SET_BOUNTY
    // listener is an explicit script-integration boundary, not an invented ACK.
    rt.lua
        .load("ShowSetupBountyWindow(50,'Wanted',12345);assert(userTrial.setupBountyWnd:IsVisible())")
        .exec()?;
    aa_ui::lua::flush(&rt.lua);
    crate::hud::expansion::fixture::click(&rt, "userTrial.setupBountyWnd.okButton")?;
    assert_eq!(
        commands(&state),
        [(
            47,
            BountyCommand::Update {
                kind: 50,
                name: b"Wanted".to_vec(),
                money: 0
            }
        )]
    );
    assert_eq!(state.borrow().trial.bounty.received.by_id[&50].money, 12345);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}
