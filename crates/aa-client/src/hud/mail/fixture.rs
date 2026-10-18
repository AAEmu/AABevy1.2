//! Original mailbox on isolated, explicit fixture state; no network connection.
use std::cell::RefCell;
use std::rc::Rc;

use aa_net::mail::{CachedMailBody, MailAttachment};
use bevy::window::PrimaryWindow;

use super::*;

pub(crate) fn run() {
    let (width, height) = aa_dev::DevArgs::from_env().size;
    let mut rt = UiRuntime::new("en_us", width as f32, height as f32).unwrap();
    rt.load_ui_texts(std::path::Path::new(aa_anim::db::DEFAULT_DB)).unwrap();
    let bridge = Rc::new(RefCell::new(super::super::api::HudData::default()));
    let game = super::super::data::GameData::load(aa_anim::db::DEFAULT_DB).unwrap();
    let item_type = *game
        .items
        .iter()
        .filter(|(_, i)| i.named.name == "Heart's Beat")
        .map(|(id, _)| id)
        .min()
        .expect("actual Heart's Beat template");
    {
        let mut d = bridge.borrow_mut();
        d.snapshot.unit_id = "100".into();
        d.snapshot.name = "Mailbox fixture".into();
        d.snapshot.level = 1;
        d.snapshot.hp = 100;
        d.snapshot.max_hp = 100;
        let info = &game.items[&item_type];
        d.panels.bag_slots = 50;
        d.panels.bag = vec![Some(super::super::panels::SlotSnap {
            kind: "item",
            id: item_type,
            uid: 123456789,
            name: info.named.name.clone(),
            icon: info.named.icon.clone(),
            count: 4,
            stack: true,
            grade: super::super::store::ui_grade(0),
            desc: info.desc.clone(),
            level: info.level,
            group: info.group,
            flags: Some(0),
            ..default()
        })];
        assert!(d.game.0.set(game).is_ok());
    }
    super::super::api::install(&mut rt, &bridge).unwrap();
    rt.run_stage(aa_ui::Stage::World);
    assert!(rt.errors.is_empty(), "{:?}", rt.errors);
    super::super::api::enter_world(&rt, &bridge, &aa_net::state::Chat::default());
    let mut wire = aa_net::wire::Writer::new();
    wire.u32(item_type)
        .u64(123456789)
        .u8(0)
        .u8(0)
        .i32(4)
        .u8(0)
        .i64(0)
        .i32(0)
        .u32(0)
        .u8(1)
        .i64(0)
        .i64(0);
    let attachment = MailAttachment::from_wire(wire.as_bytes()).unwrap();
    let state = rt.lua.app_data_ref::<State>().unwrap().clone();
    {
        let mut v = state.0.borrow_mut();
        v.nearest = 42;
        v.mailbox.received.push(MailHeader {
            mail_id: 123,
            mail_type: 2,
            title: "Supplies from a friend".into(),
            sender_name: "Peer".into(),
            receiver_name: "Mailbox fixture".into(),
            attachments: 2,
            status: 1,
            ..default()
        });
        let mut body = MailBody {
            mail_id: 123,
            mail_type: 2,
            title: "Supplies from a friend".into(),
            receiver_name: "Mailbox fixture".into(),
            text: "Here are four Heart's Beat potions and 5 copper.\n\nThis isolated fixture uses the original mailbox scripts, item data and controls.".into(),
            money: 5,
            send_date: 1_792_276_000,
            recv_date: 1_792_276_001,
            ..default()
        };
        body.items[0] = Some(attachment.clone());
        v.mailbox.bodies.push(CachedMailBody { is_sent: false, body });
        v.inventory.bag = vec![Some(attachment.item)];
        v.selected = Some((false, 123));
    }
    toggle(&rt);
    state.0.borrow_mut().selected = Some((false, 123));
    refresh(&rt, false, false, 3);
    rt.lua.load("GetReadMailWindow():SetIsReceivedMail(true)").exec().unwrap();
    refresh(&rt, false, true, 3);
    if std::env::args().any(|a| a == "--compose") {
        let widget: mlua::Value = rt.lua.load("return mailFrame.writeButton").eval().unwrap();
        let id = aa_ui::lua::value_id(&rt.lua, &widget).unwrap();
        aa_ui::lua::call_handler(&rt.lua, id, "OnClick", vec![mlua::Value::String(rt.lua.create_string("LeftButton").unwrap())]);
        rt.lua.load("local w=GetWriteMailWindow(); w.roleEdit:SetText('Peer'); w.titleEdit:SetText('Supplies for the road'); w.content:SetText([[Four potions for your journey.\n\nOriginal text entry and attachment controls.]]); assert(X2Mail:PlaceMailItemFromBag(1,1,1))").exec().unwrap();
    }
    App::new()
        .add_plugins((aa_dev::default_plugins("aa-client: original mailbox"), aa_dev::DevPlugin))
        .add_plugins(aa_ui::UiPlugin {
            config: Some(crate::login::host::ui_config()),
        })
        .insert_non_send(rt)
        .add_systems(Startup, physical_pixels)
        .add_systems(Update, paint.before(aa_ui::UiFrame))
        .run();
}

fn physical_pixels(mut windows: Query<&mut Window, With<PrimaryWindow>>) {
    for mut window in &mut windows {
        let (width, height) = aa_dev::DevArgs::from_env().size;
        window.resolution.set_scale_factor_override(Some(1.0));
        window.resolution.set_physical_resolution(width, height);
    }
}

fn paint(rt: NonSend<UiRuntime>) {
    let state = rt.lua.app_data_ref::<State>().unwrap().clone();
    let changes = std::mem::take(&mut state.0.borrow_mut().draft_changes);
    for index in changes {
        rt.fire_event("MAIL_WRITE_ITEM_UPDATE", index);
    }
}
