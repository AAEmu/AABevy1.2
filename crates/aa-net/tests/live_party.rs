//! Opt-in two-person party flow on the bevy lane; never uses an existing user's account.
//! Run: cargo test -p aa-net --test live_party -- --ignored --nocapture

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aa_net::model::{chat_type, gender, race};
use aa_net::{
    Client, Command, CreateCharacter, Event, LeaveTarget, LootingRule, MoveCommand, PartyCommand, PartyEvent,
    WorldState,
};
use anyhow::{Context, Result, bail, ensure};

struct Peer {
    client: Client,
    state: WorldState,
}

impl Peer {
    fn send(&self, command: PartyCommand) {
        assert!(self.client.send(Command::Party(command)));
    }

    async fn until(&mut self, what: &str, pick: impl Fn(&Event, &WorldState) -> bool) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(25), async {
            while let Some(event) = self.client.recv().await {
                self.state.apply(&event);
                match &event {
                    Event::LoginFailed { message }
                    | Event::ConnectFailed { message }
                    | Event::Disconnected { message } => bail!("{what}: {message}"),
                    Event::ServerError { code } => bail!("{what}: server error {code}"),
                    Event::CharacterCreateFailed { reason } => bail!("{what}: character creation failed {reason}"),
                    Event::EnterWorldDenied { reason } => bail!("{what}: world denied {reason}"),
                    Event::Party(PartyEvent::RequestRejected(message)) => bail!("{what}: {message}"),
                    _ => {}
                }
                if pick(&event, &self.state) {
                    return Ok(());
                }
            }
            bail!("{what}: session ended")
        })
        .await
        .with_context(|| format!("{what}: timed out"))?
    }

    async fn login(stamp: u128, suffix: &str) -> Result<Self> {
        let user = format!("netparty{stamp}{suffix}");
        let mut peer =
            Self { client: Client::spawn_on(&tokio::runtime::Handle::current()), state: WorldState::default() };
        peer.client.send(Command::Login { addr: aa_net::default_login(), user: user.clone(), password: user.into() });
        peer.until("world list", |e, _| matches!(e, Event::WorldList(_))).await?;
        let world_id = peer.state.worlds.list.worlds.iter().find(|w| w.online).context("no online world")?.id;
        peer.client.send(Command::EnterWorld { world_id });
        peer.until("cookie", |e, _| matches!(e, Event::ReadyToConnect { .. })).await?;
        peer.client.send(Command::ConnectToWorld);
        peer.until("characters", |e, _| matches!(e, Event::CharacterList(_))).await?;
        peer.client.send(Command::CreateCharacter(CreateCharacter::new(
            format!("P{suffix}{stamp}"),
            race::NUIAN,
            gender::MALE,
        )));
        peer.until("create character", |e, _| matches!(e, Event::CharacterList(list) if !list.is_empty())).await?;
        peer.client.send(Command::EnterGame { character_id: peer.state.characters.list[0].id });
        peer.until("spawn", |e, _| matches!(e, Event::EnteredWorld)).await?;
        println!(
            "spawned {} id={} obj={}",
            peer.state.stats.name, peer.state.stats.character_id, peer.state.stats.obj_id
        );
        Ok(peer)
    }

    async fn logout(&mut self) -> Result<()> {
        self.client.send(Command::LeaveWorld(LeaveTarget::Quit));
        self.until("logout", |e, _| matches!(e, Event::LeftWorld { .. })).await
    }
}

async fn raid_exercise(a: &mut Peer, b: &mut Peer) -> Result<()> {
    let (a_id, b_id) = (a.state.stats.character_id, b.state.stats.character_id);
    let team_id = form(a, b).await?;
    a.send(PartyCommand::SetRole { team_id, member_id: a_id, role: 1 });
    b.send(PartyCommand::SetRole { team_id, member_id: b_id, role: 2 });
    for peer in [&mut *a, &mut *b] {
        peer.until("self-selected roles", |_, s| {
            s.party.member(a_id).is_some_and(|m| m.role == 1) && s.party.member(b_id).is_some_and(|m| m.role == 2)
        })
        .await?;
    }
    a.send(PartyCommand::ConvertToRaid { team_id });
    a.until("owner raid conversion", |_, s| s.party.id == team_id && !s.party.is_party).await?;
    b.until("member raid conversion", |_, s| s.party.id == team_id && !s.party.is_party).await?;
    println!("raid: converted and both self-selected roles agree");

    a.send(a.state.party.plan_move(1, 9).context("missing source for sparse move")?);
    for peer in [&mut *a, &mut *b] {
        peer.until("move to empty slot", |_, s| s.party.member(b_id).is_some_and(|m| m.slot == 9)).await?;
    }
    a.send(a.state.party.plan_move(0, 9).context("missing source for swap")?);
    for peer in [&mut *a, &mut *b] {
        peer.until("swap occupied slots", |_, s| {
            s.party.member(a_id).is_some_and(|m| m.slot == 9 && m.role == 1)
                && s.party.member(b_id).is_some_and(|m| m.slot == 0 && m.role == 2)
        })
        .await?;
    }
    println!("raid slots: moved into another group's empty slot then swapped, preserving roles");

    let rule = LootingRule { method: 2, minimum_grade: 4, loot_master: b_id, roll_for_bind_on_pickup: false };
    a.send(PartyCommand::ChangeLootingRule { team_id, rule, flags: 15 });
    for peer in [&mut *a, &mut *b] {
        peer.until("master loot rules", |_, s| s.party.looting_rule == rule).await?;
    }
    a.send(PartyCommand::ChangeLootingRule { team_id, rule: LootingRule { method: 1, ..rule }, flags: 1 });
    for peer in [&mut *a, &mut *b] {
        peer.until("rotate clears master", |_, s| {
            s.party.looting_rule == LootingRule { method: 1, loot_master: 0, ..rule }
        })
        .await?;
    }
    println!("loot: all four settings confirmed; rotate clears loot master on the server");

    a.send(PartyCommand::SetMarker { team_id, index: 0, kind: 1, id: b_id });
    for peer in [&mut *a, &mut *b] {
        peer.until("character marker", |_, s| s.party.marks[0] == (1, b_id)).await?;
    }
    let pos = b.state.player.spawn.as_ref().context("missing ping position")?.unit.pos;
    b.send(PartyCommand::SetPing { team_id, has_ping: true, pos, instance_id: 0 });
    for peer in [&mut *a, &mut *b] {
        peer.until("marked member ping", |_, s| s.party.ping == Some((pos, 0))).await?;
    }
    let obj_id = b.state.stats.obj_id;
    a.send(PartyCommand::SetMarker { team_id, index: 1, kind: 2, id: obj_id });
    for peer in [&mut *a, &mut *b] {
        peer.until("object marker", |_, s| s.party.marks[1] == (2, obj_id)).await?;
    }
    a.send(PartyCommand::SetMarker { team_id, index: 1, kind: 0, id: 0 });
    for peer in [&mut *a, &mut *b] {
        peer.until("clear one marker", |_, s| s.party.marks[1] == (0, 0) && s.party.marks[0] == (1, b_id)).await?;
    }
    a.send(PartyCommand::SetMarker { team_id, index: -2, kind: 0, id: 0 });
    for peer in [&mut *a, &mut *b] {
        peer.until("reset all markers", |_, s| s.party.marks == [(0, 0); 12]).await?;
    }
    a.send(PartyCommand::SetPing { team_id, has_ping: false, pos, instance_id: 0 });
    for peer in [&mut *a, &mut *b] {
        peer.until("clear ping", |_, s| s.party.ping.is_none()).await?;
    }
    println!("marks/ping: character and object markers, marked-member ping, one/all clear confirmed");

    a.send(PartyCommand::Dismiss { team_id });
    for peer in [a, b] {
        peer.until("dismiss raid", |e, s| matches!(e, Event::Party(PartyEvent::Dismissed { .. })) && s.party.id == 0)
            .await?;
        ensure!(peer.state.party.members.is_empty() && peer.state.party.marks == [(0, 0); 12]);
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires the bevy lane; creates two throw-away accounts/characters"]
async fn raid_roles_sparse_moves_loot_markers_and_pings() -> Result<()> {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let mut a = Peer::login(stamp, "ra").await?;
    let mut b = match Peer::login(stamp, "rb").await {
        Ok(peer) => peer,
        Err(error) => {
            let _ = a.logout().await;
            return Err(error);
        }
    };
    let result = raid_exercise(&mut a, &mut b).await;
    let (out_a, out_b) = tokio::join!(a.logout(), b.logout());
    result?;
    out_a?;
    out_b?;
    println!("both raid test clients logged out cleanly");
    Ok(())
}

async fn form(a: &mut Peer, b: &mut Peer) -> Result<u32> {
    a.send(PartyCommand::Invite { team_id: 0, is_party: true, target_name: b.state.stats.name.clone() });
    b.until("invited", |e, _| matches!(e, Event::Party(PartyEvent::Invited(_)))).await?;
    let invite = b.state.party.pending_invite.clone().context("no invitation")?;
    ensure!(invite.sender_id == a.state.stats.character_id);
    b.send(PartyCommand::Reply { invite, accept: true });
    b.until("member joined", |e, s| matches!(e, Event::Party(PartyEvent::Joined(_))) && s.party.members.len() == 2)
        .await?;
    a.until("owner joined", |e, s| matches!(e, Event::Party(PartyEvent::Joined(_))) && s.party.members.len() == 2)
        .await?;
    let id = a.state.party.id;
    ensure!(id != 0 && b.state.party.id == id);
    ensure!(
        a.state.party.owner_id == a.state.stats.character_id && b.state.party.owner_id == a.state.stats.character_id
    );
    ensure!(b.state.party.pending_invite.is_none());
    for peer in [a, b] {
        ensure!(peer.state.party.members.iter().all(|m| m.online && m.obj_id != 0 && !m.name.is_empty()));
        ensure!(peer.state.party.member(peer.state.stats.character_id).is_some());
    }
    println!("party {id}: invited/accepted, both members and leader agree");
    Ok(id)
}

async fn exercise(a: &mut Peer, b: &mut Peer) -> Result<()> {
    let a_id = a.state.stats.character_id;
    let b_id = b.state.stats.character_id;
    // Reject before creating a party. The sender receives the rejected character's name.
    a.send(PartyCommand::Invite { team_id: 0, is_party: true, target_name: b.state.stats.name.clone() });
    b.until("decline invitation", |e, _| matches!(e, Event::Party(PartyEvent::Invited(_)))).await?;
    let invite = b.state.party.pending_invite.clone().context("missing invitation")?;
    b.send(PartyCommand::Reply { invite, accept: false });
    a.until(
        "rejected",
        |e, _| matches!(e, Event::Party(PartyEvent::Rejected { name, is_party: true }) if name == &b.state.stats.name),
    )
    .await?;
    ensure!(a.state.party.id == 0 && b.state.party.id == 0);
    println!("decline: sender received rejection, neither joined");

    let team_id = form(a, b).await?;
    let mut pos = b.state.player.spawn.as_ref().context("no spawn")?.unit.pos;
    pos.x += 1.0;
    let moved_id = b.state.stats.character_id;
    b.client.send(Command::Move(MoveCommand { pos, moving: true, ..MoveCommand::default() }));
    a.until("member position/vitals", |_, s| {
        s.party.member(moved_id).is_some_and(|m| m.has_status && m.max_hp > 0 && (m.pos.x - pos.x).abs() < 0.02)
    })
    .await?;
    let moved = a.state.party.member(moved_id).unwrap();
    ensure!(moved.hp > 0 && moved.mp > 0 && moved.max_hp >= moved.hp && moved.max_mp >= moved.mp);
    println!(
        "member sync: {} hp={}/{} mp={}/{} pos={:?}",
        moved.name, moved.hp, moved.max_hp, moved.mp, moved.max_mp, moved.pos
    );

    a.client.send(Command::Chat {
        chat_type: chat_type::PARTY,
        target: String::new(),
        message: "party protocol check".into(),
    });
    b.until(
        "party chat",
        |e, _| matches!(e, Event::Chat(m) if m.chat_type == chat_type::PARTY && m.message == "party protocol check"),
    )
    .await?;
    println!("party chat: received by the other member");

    a.send(PartyCommand::MakeOwner { team_id, member_id: b.state.stats.character_id });
    a.until("leader transfer a", |_, s| s.party.owner_id == b_id).await?;
    b.until("leader transfer b", |_, s| s.party.owner_id == b_id).await?;
    b.send(PartyCommand::MakeOwner { team_id, member_id: a.state.stats.character_id });
    a.until("leader restore a", |_, s| s.party.owner_id == a_id).await?;
    b.until("leader restore b", |_, s| s.party.owner_id == a_id).await?;
    a.send(PartyCommand::Kick { team_id, member_id: b.state.stats.character_id });
    b.until("kicked", |e, s| matches!(e, Event::Party(PartyEvent::Left { kicked: true, .. })) && s.party.id == 0)
        .await?;
    a.until("kick disbands pair", |_, s| s.party.id == 0).await?;
    println!(
        "leadership and kick: leader transferred twice; kicked member and remaining owner leave the disbanded pair"
    );

    let team_id = form(a, b).await?;
    b.send(PartyCommand::Leave { team_id, character_id: b.state.stats.character_id });
    b.until("voluntary leave", |e, s| {
        matches!(e, Event::Party(PartyEvent::Left { kicked: false, .. })) && s.party.id == 0
    })
    .await?;
    a.until("leave disbands pair", |_, s| s.party.id == 0).await?;
    println!("leave: voluntary leave clears both clients after auto-disband");
    Ok(())
}

#[tokio::test]
#[ignore = "requires the bevy lane; creates two throw-away accounts/characters"]
async fn invite_decline_accept_move_chat_leader_kick_leave() -> Result<()> {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let mut a = Peer::login(stamp, "a").await?;
    let mut b = match Peer::login(stamp, "b").await {
        Ok(peer) => peer,
        Err(e) => {
            let _ = a.logout().await;
            return Err(e);
        }
    };
    let result = exercise(&mut a, &mut b).await;
    // Always release our own sessions, even when the assertion failed.
    let (out_a, out_b) = tokio::join!(a.logout(), b.logout());
    result?;
    out_a?;
    out_b?;
    println!("both test clients logged out cleanly");
    Ok(())
}
