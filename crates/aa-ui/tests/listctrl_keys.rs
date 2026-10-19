//! `ListCtrl:InsertData(key, col, text)` takes a script-chosen key (a portal id, a mail id), not a
//! row number: a new key takes the lowest free row, a known key updates its row.

use aa_ui::{UiRuntime, lua, tree::Id};

fn exec(ui: &UiRuntime, text: &str) {
    ui.exec("@listctrl_keys", text).unwrap();
}
fn id(ui: &UiRuntime, text: &str) -> Id {
    let value = ui.lua.load(text).eval().unwrap();
    lua::value_id(&ui.lua, &value).unwrap()
}
fn setup() -> UiRuntime {
    let ui = UiRuntime::new("en_us", 1280., 720.).unwrap();
    exec(
        &ui,
        "list=UIParent:CreateWidget('listctrl','keyList','UIParent');list:SetExtent(394,440);\
         list:AddAnchor('TOPLEFT',UIParent,20,30);list:InsertColumn(190,0);list:InsertColumn(200,0);\
         list:InsertRows(4,false)",
    );
    ui
}
/// Rows (1-based) whose sub-items show.
fn filled(ui: &UiRuntime) -> Vec<usize> {
    let list = id(ui, "list");
    let u = lua::ui(&ui.lua);
    let l = u.tree.widget(list).unwrap().list.as_ref().unwrap();
    (0..l.rows.len()).filter(|&i| l.has_data[i]).map(|i| i + 1).collect()
}
fn text(ui: &UiRuntime, row: usize, col: usize) -> String {
    let sub = id(ui, &format!("list.items[{row}].subItems[{col}]"));
    lua::ui(&ui.lua).tree.get(sub).text().unwrap().text.clone()
}
fn shown(ui: &UiRuntime, row: usize, col: usize) -> bool {
    let sub = id(ui, &format!("list.items[{row}].subItems[{col}]"));
    lua::ui(&ui.lua).tree.get(sub).shown
}

#[test]
fn new_keys_take_rows_in_insertion_order() {
    let ui = setup();
    exec(&ui, "list:InsertData(243,1,'a');list:InsertData(4096,1,'b')");
    assert_eq!(filled(&ui), vec![1, 2]);
    assert_eq!((text(&ui, 1, 1), text(&ui, 2, 1)), ("a".into(), "b".into()));
    assert!(shown(&ui, 1, 1) && shown(&ui, 2, 2) && !shown(&ui, 3, 1));
    // Another column of a known key lands on its row, not a new one.
    exec(&ui, "list:InsertData(243,2,'c');list:InsertData(7,1,'d')");
    assert_eq!(filled(&ui), vec![1, 2, 3]);
    assert_eq!((text(&ui, 1, 2), text(&ui, 3, 1)), ("c".into(), "d".into()));
}

#[test]
fn known_key_updates_its_row() {
    let ui = setup();
    exec(&ui, "list:InsertData(9,1,'x');list:InsertData(5,1,'y');list:InsertData(9,1,'z')");
    assert_eq!(filled(&ui), vec![1, 2]);
    assert_eq!((text(&ui, 1, 1), text(&ui, 2, 1)), ("z".into(), "y".into()));
    // String keys work the same.
    exec(&ui, "list:InsertData('bob',1,'1');list:InsertData('bob',2,'2')");
    assert_eq!((filled(&ui), text(&ui, 3, 1), text(&ui, 3, 2)), (vec![1, 2, 3], "1".into(), "2".into()));
}

#[test]
fn sequential_row_numbers_still_land_on_their_rows() {
    let ui = setup();
    exec(&ui, "for i=1,4 do list:InsertData(i,1,'r'..i);list:InsertData(i,2,'s'..i) end");
    assert_eq!(filled(&ui), vec![1, 2, 3, 4]);
    for i in 1..=4 {
        assert_eq!((text(&ui, i, 1), text(&ui, i, 2)), (format!("r{i}"), format!("s{i}")));
    }
    // A full list ignores a fifth key.
    exec(&ui, "list:InsertData(99,1,'no')");
    assert_eq!(filled(&ui), vec![1, 2, 3, 4]);
}

#[test]
fn delete_by_key_frees_the_row_for_the_next_key() {
    let ui = setup();
    exec(&ui, "list:InsertData(10,1,'a');list:InsertData(20,1,'b');list:InsertData(30,1,'c')");
    exec(&ui, "list:DeleteData(20)");
    assert_eq!(filled(&ui), vec![1, 3]);
    assert!(!shown(&ui, 2, 1));
    exec(&ui, "list:DeleteData(12345)");
    assert_eq!(filled(&ui), vec![1, 3]);
    exec(&ui, "list:InsertData(40,1,'d')");
    assert_eq!((filled(&ui), text(&ui, 2, 1)), (vec![1, 2, 3], "d".into()));
    // DeleteAllDatas forgets every key.
    exec(&ui, "list:DeleteAllDatas();list:InsertData(30,1,'e')");
    assert_eq!((filled(&ui), text(&ui, 1, 1)), (vec![1], "e".into()));
}

#[test]
fn selection_is_by_row_and_survives_keyed_inserts() {
    let ui = setup();
    exec(&ui, "list:InsertData(243,1,'a');list:InsertData(4096,1,'b');list:Select(1)");
    assert_eq!(ui.lua.load("return list:GetSelectedIdx()").eval::<i64>().unwrap(), 2);
    exec(&ui, "list:ClearSelection()");
    assert_eq!(ui.lua.load("return list:GetSelectedIdx()").eval::<i64>().unwrap(), 0);
}

#[test]
fn delete_rows_drops_the_rows_and_keys() {
    let ui = setup();
    exec(&ui, "list:InsertData(1,1,'a');list:DeleteRows();list:InsertRows(2,false);list:InsertData(1,1,'b')");
    assert_eq!((filled(&ui), text(&ui, 1, 1)), (vec![1], "b".into()));
    assert_eq!(ui.lua.load("return #list.items").eval::<i64>().unwrap(), 2);
}
