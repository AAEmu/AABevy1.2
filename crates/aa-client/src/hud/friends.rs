//! Original relationship tabs over acknowledged friends/blocked users, with local pagination.
use std::{cell::RefCell, rc::Rc};

use aa_net::{
    Friends,
    friend::{FriendCommand, FriendEvent, FriendInfo},
};
use aa_ui::UiRuntime;
use bevy::prelude::*;
use mlua::{Function, Lua, Table};

use super::{
    api::{HudBridge, Request, Shared},
    data::GameData,
};

// Native's runtime-configured page-size global is unrecovered. This explicit app page
// size matches the ten visible rows; it is not claimed to be the native global value.
const PER_PAGE: usize = 10;

#[derive(Clone, Default)]
struct Queue(Rc<RefCell<View>>);

#[derive(Default)]
struct View {
    page: Option<(bool, usize)>,
    requested: bool,
    seen: Option<Friends>,
    notices: Vec<(&'static str, &'static str, &'static str, String)>,
    waiting: Vec<(FriendCommand, f64)>,
}

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        sync.after(super::sync_hud)
            .before(super::api::handle_requests)
            .run_if(super::hud_up)
            .run_if(in_state(crate::Screen::InGame)),
    );
}

pub(super) fn install(lua: &Lua, bridge: &Shared) -> mlua::Result<()> {
    let q = Queue::default();
    lua.set_app_data(q.clone());
    let friend: Table = lua.globals().get("X2Friend")?;
    for (method, blocked) in [("IsMyFriend", false), ("IsBlockedUser", true)] {
        let b = bridge.clone();
        friend.set(
            method,
            lua.create_function(move |_, (_, name): (mlua::Value, String)| {
                let d = b.borrow();
                Ok(if blocked {
                    d.friends.is_blocked(&name)
                } else {
                    d.friends
                        .friends
                        .iter()
                        .any(|f| f.name.eq_ignore_ascii_case(&name))
                })
            })?,
        )?;
    }
    let b = bridge.clone();
    friend.set(
        "GetFriendList",
        lua.create_function(move |lua, (_, show_offline): (mlua::Value, Option<bool>)| {
            let d = b.borrow();
            let mut friends: Vec<_> = d
                .friends
                .friends
                .iter()
                .filter(|f| show_offline.unwrap_or(false) || f.online)
                .collect();
            friends.sort_by_key(|f| std::cmp::Reverse(f.level));
            let rows = friends
                .into_iter()
                .map(|f| row(lua, f, d.game.get()))
                .collect::<mlua::Result<Vec<_>>>()?;
            lua.create_sequence_from(rows)
        })?,
    )?;
    friend.set(
        "GetFriendsPerPage",
        lua.create_function(|_, _: mlua::Value| Ok(PER_PAGE))?,
    )?;
    let b = bridge.clone();
    friend.set(
        "GetFriendCount",
        lua.create_function(move |_, _: mlua::Value| {
            let d = b.borrow();
            Ok((
                d.friends.friends.iter().filter(|f| f.online).count(),
                d.friends.total,
            ))
        })?,
    )?;
    let b = bridge.clone();
    friend.set(
        "GetBlockList",
        lua.create_function(move |lua, _: mlua::Value| {
            let d = b.borrow();
            let rows = lua.create_table()?;
            for (i, user) in d.friends.blocked.iter().enumerate() {
                rows.set(i + 1, lua.create_sequence_from([user.name.as_str()])?)?;
            }
            Ok((rows, d.friends.blocked.len()))
        })?,
    )?;
    let query = q.clone();
    friend.set(
        "RequestFriendList",
        lua.create_function(
            move |_, (_, show_offline, start): (mlua::Value, bool, Option<usize>)| {
                let mut view = query.0.borrow_mut();
                view.page = Some((show_offline, start.unwrap_or(1).saturating_sub(1)));
                view.requested = true;
                Ok(())
            },
        )?,
    )?;
    for (method, kind) in [
        ("AddFriend", 0),
        ("DeleteFriend", 1),
        ("BlockUser", 2),
        ("UnblockUser", 3),
    ] {
        let b = bridge.clone();
        let queue = q.clone();
        friend.set(
            method,
            lua.create_function(move |_, (_, name): (mlua::Value, String)| {
                let name = name.trim().to_string();
                let command = match kind {
                    0 => FriendCommand::Add { name: name.clone() },
                    1 => FriendCommand::Remove { name: name.clone() },
                    2 => FriendCommand::Block { name: name.clone() },
                    _ => FriendCommand::Unblock { name: name.clone() },
                };
                if command.validate().is_err() {
                    return Ok(());
                }
                let mut d = b.borrow_mut();
                let own = d.snapshot.name.eq_ignore_ascii_case(&name);
                let exists = d
                    .friends
                    .friends
                    .iter()
                    .any(|f| f.name.eq_ignore_ascii_case(&name));
                let blocked = d.friends.is_blocked(&name);
                let notice = match kind {
                    0 if own => Some(("FRIENDLIST", "ERROR_MSG", "CANNOT_ADD_FRIEND_SELF")),
                    0 if exists => Some(("FRIENDLIST", "ERROR_MSG", "ADD_FRIEND")),
                    1 if !exists => Some(("FRIENDLIST", "ERROR_MSG", "CANNOT_FIND_IN_FRIEND_LIST")),
                    2 if own => Some(("BLOCKED_USER_LIST", "COMMUNITY_TEXT", "cannot_block_self")),
                    2 if blocked => Some((
                        "BLOCKED_USER_LIST",
                        "COMMUNITY_TEXT",
                        "user_already_blocked",
                    )),
                    3 if !blocked => {
                        Some(("BLOCKED_USER_LIST", "COMMUNITY_TEXT", "user_not_in_list"))
                    }
                    _ => None,
                };
                if let Some((event, group, key)) = notice {
                    // Local preflight notice, not a fabricated network response.
                    queue.0.borrow_mut().notices.push((event, group, key, name));
                } else if !d.requests.contains(&Request::Friend(command.clone())) {
                    queue
                        .0
                        .borrow_mut()
                        .waiting
                        .push((command.clone(), d.start.elapsed().as_secs_f64()));
                    d.requests.push(Request::Friend(command));
                }
                Ok(())
            })?,
        )?;
    }
    Ok(())
}

fn localize(lua: &Lua, group: &str, key: &str, name: &str) -> mlua::Result<String> {
    let locale: Table = lua.globals().get("X2Locale")?;
    let f: Function = locale.get("LocalizeUiText")?;
    let group: i64 = lua.globals().get(group)?;
    f.call((locale, group, key, name))
}

fn row(lua: &Lua, friend: &FriendInfo, game: Option<&GameData>) -> mlua::Result<Table> {
    let t = lua.create_table()?;
    t.set(1, friend.name.as_str())?;
    t.set(2, friend.level)?;
    t.set(3, lua.create_sequence_from(friend.abilities)?)?;
    // English's original view does not draw column4. Native offline elapsed-duration
    // formatting is pending; retain localized Offline instead of inventing a date.
    let place = if friend.online {
        game.and_then(|g| {
            g.zone(friend.zone_id)
                .map(|(_, z, _)| z.text.clone())
                .or_else(|| g.zone_by_group(friend.zone_id).map(|(z, _)| z.text.clone()))
        })
        .unwrap_or_default()
    } else {
        localize(lua, "UNIT_FRAME_TEXT", "offline", "").unwrap_or_default()
    };
    t.set(4, place)?;
    t.set(5, friend.race)?;
    t.set(6, friend.online)?;
    t.set(7, friend.in_party)?;
    Ok(t)
}

fn page(friends: &Friends, show_offline: bool, start: usize) -> (usize, usize, Vec<&FriendInfo>) {
    let mut rows: Vec<_> = friends
        .friends
        .iter()
        .filter(|f| show_offline || f.online)
        .collect();
    // Native merge39302f90 compares level descending and preserves ties.
    rows.sort_by_key(|f| std::cmp::Reverse(f.level));
    let count = rows.len();
    let start = if count == 0 {
        0
    } else {
        start.min((count - 1) / PER_PAGE * PER_PAGE)
    };
    (
        count,
        start,
        rows.into_iter().skip(start).take(PER_PAGE).collect(),
    )
}

fn sync(
    rt: Option<NonSend<UiRuntime>>,
    bridge: NonSend<HudBridge>,
    mut events: MessageReader<aa_net::NetEvent>,
) {
    let signals: Vec<_> = events
        .read()
        .filter_map(|aa_net::NetEvent(e)| {
            if let aa_net::Event::Friend(e) = e {
                Some(e.clone())
            } else {
                None
            }
        })
        .collect();
    let Some(rt) = rt else { return };
    let Some(queue) = rt.lua.app_data_ref::<Queue>().map(|q| q.clone()) else {
        return;
    };
    let (friends, game) = {
        let d = bridge.0.borrow();
        (d.friends.clone(), d.game.clone())
    };
    let now = bridge.0.borrow().start.elapsed().as_secs_f64();
    let mut prior_online = if signals.is_empty() {
        Vec::new()
    } else {
        queue
            .0
            .borrow()
            .seen
            .as_ref()
            .map(|s| s.friends.iter().map(|f| (f.id, f.online)).collect())
            .unwrap_or_default()
    };
    let expired = {
        let mut view = queue.0.borrow_mut();
        let mut expired = Vec::new();
        view.waiting.retain(|(command, sent)| {
            // Acknowledgements have already been reduced; complete only requests whose ACK
            // is in this batch. An empty pending list before send is not an acknowledgement.
            let complete = signals.iter().any(|e| match e {
                FriendEvent::Added { friend, .. } => same_request(
                    &FriendCommand::Add {
                        name: friend.name.clone(),
                    },
                    command,
                ),
                FriendEvent::Removed { name, .. } => {
                    same_request(&FriendCommand::Remove { name: name.clone() }, command)
                }
                FriendEvent::Blocked { user, .. } => same_request(
                    &FriendCommand::Block {
                        name: user.name.clone(),
                    },
                    command,
                ),
                FriendEvent::Unblocked { name, .. } => {
                    same_request(&FriendCommand::Unblock { name: name.clone() }, command)
                }
                FriendEvent::RequestRejected {
                    command: rejected, ..
                } => same_request(rejected, command),
                _ => false,
            });
            if complete {
                false
            } else if now - *sent >= 20.0 {
                expired.push(command.clone());
                false
            } else {
                true
            }
        });
        expired
    };
    for command in expired {
        let (event, key) = match command {
            FriendCommand::Add { .. } => ("FRIENDLIST", "ADD_FRIEND"),
            FriendCommand::Remove { .. } => ("FRIENDLIST", "DELETE_FRIEND"),
            FriendCommand::Block { .. } => ("BLOCKED_USER_LIST", "BLOCK_USER"),
            FriendCommand::Unblock { .. } => ("BLOCKED_USER_LIST", "UNBLOCK_USER"),
        };
        warn!("hud: relationship request timed out without acknowledgement: {command:?}");
        if let Ok(text) = localize(&rt.lua, "ERROR_MSG", key, command.name()) {
            rt.fire_event(event, text);
        }
    }
    let (refresh, selection, blocked, notices) = {
        let mut v = queue.0.borrow_mut();
        let refresh = std::mem::take(&mut v.requested)
            || v.seen.as_ref().is_none_or(|old| {
                old.friends != friends.friends
                    || old.total != friends.total
                    || old.loaded != friends.loaded
            });
        let blocked = v.seen.as_ref().is_none_or(|old| {
            old.blocked != friends.blocked || old.blocked_loaded != friends.blocked_loaded
        });
        v.seen = Some(friends.clone());
        (
            refresh,
            v.page.unwrap_or((false, 0)),
            blocked,
            std::mem::take(&mut v.notices),
        )
    };
    for (event, group, key, name) in notices {
        if let Ok(text) = localize(&rt.lua, group, key, &name) {
            rt.fire_event(event, text);
        }
    }
    for signal in signals {
        use FriendEvent as E;
        let notice = match signal {
            E::Added {
                friend,
                success,
                error,
            } => {
                if success {
                    if let Ok(row) = row(&rt.lua, &friend, game.get()) {
                        rt.fire_event("FRIENDLIST_UPDATE", ("insert", row));
                    }
                }
                Some((false, success, error, "friend_added", friend.name))
            }
            E::Removed {
                name,
                success,
                error,
                ..
            } => {
                if success {
                    rt.fire_event("FRIENDLIST_UPDATE", ("delete", name.as_str()));
                }
                Some((false, success, error, "friend_deleted", name))
            }
            E::StatusChanged(friend) => {
                if let Some((_, was_online)) =
                    prior_online.iter_mut().find(|(id, _)| *id == friend.id)
                {
                    if friend.online && !*was_online {
                        if let Ok(text) = localize(
                            &rt.lua,
                            "COMMUNITY_TEXT",
                            "online_acquaintance",
                            &friend.name,
                        ) {
                            rt.fire_event("FRIENDLIST", text);
                        }
                    }
                    *was_online = friend.online;
                }
                if let Ok(row) = row(&rt.lua, &friend, game.get()) {
                    rt.fire_event("FRIENDLIST_UPDATE", ("update", row));
                }
                None
            }
            E::Blocked {
                user,
                success,
                error,
            } => Some((true, success, error, "user_blocked", user.name)),
            E::Unblocked {
                name,
                success,
                error,
                ..
            } => Some((true, success, error, "user_unblocked", name)),
            E::RequestRejected { command, message } => {
                warn!("hud: friend request {:?}: {message}", command);
                None
            }
            _ => None,
        };
        if let Some((block, success, error, key, name)) = notice {
            let (group, key) = if success {
                ("COMMUNITY_TEXT", key)
            } else {
                (
                    "ERROR_MSG",
                    super::alerts::error_name(error).unwrap_or(if block {
                        "BLOCK_USER"
                    } else {
                        "ADD_FRIEND"
                    }),
                )
            };
            if let Ok(text) = localize(&rt.lua, group, key, &name) {
                rt.fire_event(
                    if block {
                        "BLOCKED_USER_LIST"
                    } else {
                        "FRIENDLIST"
                    },
                    text,
                );
            }
        }
    }
    if blocked {
        rt.fire_event("BLOCKED_USER_UPDATE", ());
    }
    if refresh {
        let (count, start, rows) = page(&friends, selection.0, selection.1);
        let rows = rows
            .into_iter()
            .map(|friend| row(&rt.lua, friend, game.get()))
            .collect::<mlua::Result<Vec<_>>>()
            .and_then(|rows| rt.lua.create_sequence_from(rows));
        match rows {
            Ok(rows) => rt.fire_event("FRIENDLIST_INFO", (count, start, rows)),
            Err(e) => warn!("hud: friend list: {e}"),
        }
    }
}

fn same_request(a: &FriendCommand, b: &FriendCommand) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
        && a.name().eq_ignore_ascii_case(b.name())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn friend(id: u32, name: &str, level: u8, online: bool) -> FriendInfo {
        FriendInfo {
            id,
            name: name.into(),
            level,
            online,
            race: 1,
            abilities: [1, 11, 11],
            ..default()
        }
    }

    #[test]
    #[ignore = "loads the original world-stage assets; run in an isolated fixture process"]
    fn original_relationship_popup_unblock_and_offline_rows()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut rt = UiRuntime::new("en_us", 1280.0, 720.0)?;
        rt.load_ui_texts(std::path::Path::new(aa_anim::db::DEFAULT_DB))?;
        let bridge = Rc::new(RefCell::new(super::super::api::HudData::default()));
        {
            let mut d = bridge.borrow_mut();
            d.snapshot.name = "Self".into();
            d.snapshot.unit_id = "100".into();
            d.snapshot.level = 1;
            d.snapshot.max_hp = 100;
            d.snapshot.hp = 100;
            assert!(
                d.game
                    .0
                    .set(GameData::load(aa_anim::db::DEFAULT_DB)?)
                    .is_ok()
            );
            d.friends = Friends {
                friends: vec![friend(12, "Friend", 20, true)],
                total: 1,
                loaded: true,
                blocked_loaded: true,
                blocked: vec![aa_net::friend::BlockedUser {
                    id: 12,
                    name: "Friend".into(),
                }],
                blocked_total: 1,
                ..default()
            };
        }
        super::super::api::install(&mut rt, &bridge)?;
        let stage_started = std::time::Instant::now();
        rt.lua.set_hook(
            mlua::HookTriggers {
                every_nth_instruction: Some(50_000),
                ..default()
            },
            move |_, debug| {
                if stage_started.elapsed().as_secs() > 6 {
                    Err(mlua::Error::runtime(format!(
                        "original stage instruction budget at {:?}:{}",
                        debug.source().source,
                        debug.current_line().unwrap_or(0)
                    )))
                } else {
                    Ok(mlua::VmState::Continue)
                }
            },
        )?;
        rt.run_stage(aa_ui::Stage::World);
        rt.lua.remove_hook();
        assert!(
            rt.errors.is_empty(),
            "original stage errors: {:?}",
            rt.errors
        );
        let mut app = App::new();
        app.add_message::<aa_net::NetEvent>();
        app.add_systems(Update, sync);
        app.insert_non_send(HudBridge(bridge.clone()));
        app.insert_non_send(rt);
        app.update();
        app.world().non_send::<UiRuntime>().lua.load("local old=CreateEmptyWindow; CreateEmptyWindow=function(id,parent,...) local w=old(id,parent,...); if id=='popupMenu' then fixturePopup=w end; return w end; ToggleRelationshipUI(true); relationshipFrame.tab:SelectTab(2); local block=relationshipFrame.tab.window[2]; assert(block.frame:GetDataCount()==1); block.frame.listCtrl.items[1]:OnClick('RightButton'); assert(fixturePopup and fixturePopup.buttons[1]); fixturePopup.buttons[1]:OnClick()").exec()?;
        assert_eq!(
            bridge.borrow_mut().requests.pop(),
            Some(Request::Friend(FriendCommand::Unblock {
                name: "Friend".into()
            }))
        );
        let unblocked = aa_net::Event::Friend(FriendEvent::Unblocked {
            id: 12,
            name: "Friend".into(),
            success: true,
            error: 0,
        });
        bridge.borrow_mut().friends.apply(&unblocked);
        app.world_mut().write_message(aa_net::NetEvent(unblocked));
        app.update();
        app.world().non_send::<UiRuntime>().lua.load("assert(relationshipFrame.tab.window[2].frame:GetDataCount()==0); relationshipFrame.tab:SelectTab(1)").exec()?;
        app.update();
        aa_ui::lua::tick(&app.world().non_send::<UiRuntime>().lua, 1100.0);
        app.world().non_send::<UiRuntime>().lua.load("local f=relationshipFrame.tab.window[1]; assert(f.refreshButton:IsEnabled()); f.showOffMember:SetChecked(true,false); f.showOffMember:CheckBtnCheckChagnedProc(true)").exec()?;
        let offline = friend(12, "Friend", 20, false);
        let event = aa_net::Event::Friend(FriendEvent::StatusChanged(offline));
        bridge.borrow_mut().friends.apply(&event);
        app.world_mut().write_message(aa_net::NetEvent(event));
        app.update();
        app.world().non_send::<UiRuntime>().lua.load("local f=relationshipFrame.tab.window[1]; local row=f.frame:GetDataByViewIndex(1,1); assert(row[1]=='Friend' and row[6]==false); local on,total=X2Friend:GetFriendCount(); assert(on==0 and total==1); f.frame.listCtrl.items[1]:OnClick('RightButton'); local clicked=false; for _,button in ipairs(fixturePopup.buttons) do if button:GetText()==locale.friend.deleteFriend then button:OnClick(); clicked=true; break end end; assert(clicked)").exec()?;
        assert_eq!(
            bridge.borrow_mut().requests.pop(),
            Some(Request::Friend(FriendCommand::Remove {
                name: "Friend".into()
            }))
        );
        assert!(app.world().non_send::<UiRuntime>().errors.is_empty());
        Ok(())
    }

    #[test]
    fn view_filters_before_stable_level_sort_and_pagination() {
        let friends = Friends {
            friends: vec![
                friend(1, "Offline", 30, false),
                friend(2, "First", 20, true),
                friend(3, "Second", 20, true),
                friend(4, "Last", 10, true),
            ],
            ..default()
        };
        let (total, start, rows) = page(&friends, false, 999);
        assert_eq!((total, start), (3, 0));
        assert_eq!(rows.iter().map(|f| f.id).collect::<Vec<_>>(), [2, 3, 4]);
        assert_eq!(page(&friends, true, 0).2[0].id, 1);
        assert_eq!(page(&Friends::default(), false, 999).0, 0);
    }

    #[test]
    fn relationship_queries_and_acknowledgements_keep_native_rows_and_names()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut rt = UiRuntime::new("en_us", 1280.0, 720.0)?;
        let bridge = Rc::new(RefCell::new(super::super::api::HudData::default()));
        {
            let mut d = bridge.borrow_mut();
            d.snapshot.name = "Self".into();
            d.friends = Friends {
                friends: vec![
                    friend(11, "Offline", 30, false),
                    friend(12, "Online", 20, true),
                ],
                total: 2,
                loaded: true,
                blocked_loaded: true,
                ..default()
            };
        }
        super::super::api::install(&mut rt, &bridge)?;
        rt.lua.load("local on,total=X2Friend:GetFriendCount(); assert(on==1 and total==2 and X2Friend:GetFriendsPerPage()>0); local b,n=X2Friend:GetBlockList(); assert(type(b)=='table' and n==0); assert(X2Friend:IsMyFriend('ONLINE') and not X2Friend:IsBlockedUser('Online')); local all=X2Friend:GetFriendList(true); assert(#all==2 and all[1][1]=='Offline'); pages={}; changes={}; UIParent:SetEventHandler('FRIENDLIST_INFO',function(total,start,rows) pages={total,start,rows} end); UIParent:SetEventHandler('FRIENDLIST_UPDATE',function(kind,row) changes={kind,row} end); X2Friend:RequestFriendList(true,1); X2Friend:AddFriend('Self'); X2Friend:AddFriend(' Newfriend ')").exec()?;
        assert_eq!(
            bridge.borrow().requests,
            [Request::Friend(FriendCommand::Add {
                name: "Newfriend".into()
            })]
        );
        assert_eq!(bridge.borrow().friends.friends.len(), 2);
        let mut app = App::new();
        app.add_message::<aa_net::NetEvent>();
        app.add_systems(Update, sync);
        app.insert_non_send(HudBridge(bridge.clone()));
        app.insert_non_send(rt);
        app.update();
        app.world().non_send::<UiRuntime>().lua.load("assert(pages[1]==2 and pages[2]==0 and #pages[3]==2); local f=pages[3][1]; assert(f[1]=='Offline' and f[2]==30 and f[3][1]==1 and f[5]==1 and f[6]==false and f[7]==false)").exec()?;
        let added = friend(13, "Newfriend", 40, true);
        bridge.borrow_mut().friends.friends.push(added.clone());
        bridge.borrow_mut().friends.total = 3;
        app.world_mut()
            .write_message(aa_net::NetEvent(aa_net::Event::Friend(
                FriendEvent::Added {
                    friend: added,
                    success: true,
                    error: 0,
                },
            )));
        app.update();
        app.world().non_send::<UiRuntime>().lua.load("assert(changes[1]=='insert' and changes[2][1]=='Newfriend'); assert(pages[3][1][1]=='Newfriend')").exec()?;
        bridge.borrow_mut().friends.friends.retain(|f| f.id != 13);
        bridge.borrow_mut().friends.total = 2;
        app.world_mut()
            .write_message(aa_net::NetEvent(aa_net::Event::Friend(
                FriendEvent::Removed {
                    id: 13,
                    name: "Newfriend".into(),
                    success: true,
                    error: 0,
                },
            )));
        app.update();
        app.world()
            .non_send::<UiRuntime>()
            .lua
            .load("assert(changes[1]=='delete' and changes[2]=='Newfriend'); assert(pages[1]==2)")
            .exec()?;
        Ok(())
    }
}
