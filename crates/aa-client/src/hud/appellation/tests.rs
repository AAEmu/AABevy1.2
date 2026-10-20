use super::*;

fn setup(owned: &[u32], active: u32) -> Result<(UiRuntime, Shared), Box<dyn std::error::Error>> {
    let (rt, state, _) = super::super::breath::fixture::runtime(1280., 720.)?;
    {
        let mut d = state.borrow_mut();
        d.character_id = 10;
        d.appellation.titles.owned = owned.to_vec();
        d.appellation.titles.active = active;
    }
    Ok((rt, state))
}

fn requests(state: &Shared) -> Vec<u32> {
    std::mem::take(&mut state.borrow_mut().appellation.requests)
}

#[test]
fn the_getters_read_the_received_list_and_the_database() -> Result<(), Box<dyn std::error::Error>> {
    // 100 and 101 are titles of the database with a buff each (5754 and 5788)
    let (rt, _state) = setup(&[100, 101], 101)?;
    rt.lua
        .load(
            r#"
        assert(X2Player:GetAppellationCount() == 2)
        local list = X2Player:GetAppellations(1)
        assert(#list == 2, #list)
        local T, N, B = APPELLATION_INFO.TYPE, APPELLATION_INFO.NAME, APPELLATION_INFO.BUFF_INFO
        assert(list[1][T] == 100 and list[2][T] == 101)
        assert(type(list[1][N]) == "string" and list[1][N] ~= "", "name")
        assert(list[1][B].path ~= "" and list[1][B].name ~= "" and list[1][B].tipType == "buff")
        local shown = X2Player:GetShowingAppellation()
        assert(shown[T] == 101 and shown[N] == list[2][N])
        assert(#X2Player:GetAppellations(2) == 0)
    "#,
        )
        .exec()?;
    Ok(())
}

#[test]
fn no_title_shown_is_nil() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, _state) = setup(&[100], 0)?;
    rt.lua.load("assert(X2Player:GetShowingAppellation() == nil); assert(X2Player:GetAppellationCount() == 1)").exec()?;
    let (rt, _state) = setup(&[], 0)?;
    rt.lua.load("assert(X2Player:GetAppellationCount() == 0); assert(#X2Player:GetAppellations(1) == 0)").exec()?;
    Ok(())
}

#[test]
fn the_pages_hold_five_titles() {
    let titles = Appellations { owned: (1..=12).collect(), ..Default::default() };
    assert_eq!((page_ids(&titles, 1).len(), page_ids(&titles, 2).len(), page_ids(&titles, 3).len()), (5, 5, 2));
    assert_eq!(page_ids(&titles, 2)[0], 6);
    assert!(page_ids(&titles, 4).is_empty());
    // a page below 1 is the first one
    assert_eq!(page_ids(&titles, 0), page_ids(&titles, 1));
    assert_eq!(page_ids(&titles, -3), page_ids(&titles, 1));
    assert!(page_ids(&Appellations::default(), 1).is_empty());
}

#[test]
fn changing_a_title_asks_the_server_only_for_an_owned_one() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup(&[100, 101], 100)?;
    rt.lua.load("assert(X2Player:ChangeAppellation(101) == true)").exec()?;
    assert_eq!(requests(&state), vec![101]);
    // take the title off
    rt.lua.load("assert(X2Player:ChangeAppellation(0) == true)").exec()?;
    assert_eq!(requests(&state), vec![0]);
    // not owned: nothing goes out
    rt.lua.load("assert(X2Player:ChangeAppellation(102) == false)").exec()?;
    assert!(requests(&state).is_empty());
    state.borrow_mut().disconnected = true;
    rt.lua.load("assert(X2Player:ChangeAppellation(101) == false)").exec()?;
    assert!(requests(&state).is_empty());
    Ok(())
}

#[test]
fn the_window_lists_the_titles_and_a_click_picks_one() -> Result<(), Box<dyn std::error::Error>> {
    let (rt, state) = setup(&[100, 101, 102], 100)?;
    rt.lua
        .load(
            r#"
        local win = CreateAppellationWindow("tw", "UIParent")
        win:Show(true)
        win:FillAppellationData(1)
        TW = win
    "#,
        )
        .exec()?;
    let _ = state;
    Ok(())
}
