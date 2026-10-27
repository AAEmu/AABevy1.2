//! Opt-in fresh paired social flow; no existing user accounts or area broadcasts.
//! cargo test -p aa-net --test live_friends -- --ignored --nocapture

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use aa_net::model::{gender, race};
use aa_net::{
    Client, Command, CreateCharacter, Event, FriendCommand, FriendEvent, LeaveTarget, PartyCommand, PartyEvent,
    WorldState,
};
use anyhow::{Context, Result, bail, ensure};

struct Peer {
    client: Client,
    state: WorldState,
    user: String,
}

impl Peer {
    fn send(&self, command: FriendCommand) {
        assert!(self.client.send(Command::Friend(command)));
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
                    Event::CharacterCreateFailed { reason } => bail!("{what}: character create {reason}"),
                    Event::EnterWorldDenied { reason } => bail!("{what}: world denied {reason}"),
                    Event::Friend(FriendEvent::RequestRejected { message, .. }) => bail!("{what}: {message}"),
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

    async fn connect(user: String, create: Option<String>) -> Result<Self> {
        let mut peer = Self {
            client: Client::spawn_on(&tokio::runtime::Handle::current()),
            state: WorldState::default(),
            user: user.clone(),
        };
        peer.client.send(Command::Login { addr: aa_net::default_login(), user: user.clone(), password: user.into() });
        peer.until("world list", |e, _| matches!(e, Event::WorldList(_))).await?;
        let world_id = peer.state.worlds.list.worlds.iter().find(|w| w.online).context("no online world")?.id;
        peer.client.send(Command::EnterWorld { world_id });
        peer.until("cookie", |e, _| matches!(e, Event::ReadyToConnect { .. })).await?;
        peer.client.send(Command::ConnectToWorld);
        peer.until("characters", |e, _| matches!(e, Event::CharacterList(_))).await?;
        if let Some(name) = create {
            peer.client.send(Command::CreateCharacter(CreateCharacter::new(name, race::NUIAN, gender::MALE)));
            peer.until("created", |e, _| matches!(e, Event::CharacterList(list) if !list.is_empty())).await?;
        }
        let id = peer.state.characters.list.first().context("missing own character")?.id;
        peer.client.send(Command::EnterGame { character_id: id });
        peer.until("spawn", |e, _| matches!(e, Event::EnteredWorld)).await?;
        ensure!(peer.state.friends.loaded && peer.state.friends.blocked_loaded);
        println!("online {} id={id}", peer.state.stats.name);
        Ok(peer)
    }

    async fn fresh(stamp: u128, suffix: &str) -> Result<Self> {
        Self::connect(format!("netfriend{stamp}{suffix}"), Some(format!("F{suffix}{stamp}"))).await
    }

    async fn logout(&mut self) -> Result<()> {
        if self.state.player.spawn.is_none() {
            return Ok(());
        }
        self.client.send(Command::LeaveWorld(LeaveTarget::Quit));
        self.until("logout", |e, _| matches!(e, Event::LeftWorld { .. })).await
    }

    async fn reconnect(&mut self) -> Result<()> {
        ensure!(self.state.player.spawn.is_none(), "must log out own session before reconnecting");
        *self = Self::connect(self.user.clone(), None).await?;
        Ok(())
    }
}

async fn exercise(a: &mut Peer, b: &mut Peer) -> Result<()> {
    let (a_id, b_id) = (a.state.stats.character_id, b.state.stats.character_id);
    let (a_name, b_name) = (a.state.stats.name.clone(), b.state.stats.name.clone());
    ensure!(a.state.friends.friends.is_empty() && a.state.friends.blocked.is_empty());
    a.send(FriendCommand::Add { name: b_name.clone() });
    a.until("add friend A", |_, s| s.friends.friend(b_id).is_some_and(|f| f.online)).await?;
    b.send(FriendCommand::Add { name: a_name.clone() });
    b.until("add friend B", |_, s| s.friends.friend(a_id).is_some_and(|f| f.online)).await?;
    ensure!(a.state.friends.pending.is_empty() && b.state.friends.pending.is_empty());
    println!("friends: mutual add acknowledged with online status");

    a.client.send(Command::Party(PartyCommand::Invite { team_id: 0, is_party: true, target_name: b_name.clone() }));
    b.until("party invite", |e, _| matches!(e, Event::Party(PartyEvent::Invited(_)))).await?;
    let invite = b.state.party.pending_invite.clone().context("missing invite")?;
    b.client.send(Command::Party(PartyCommand::Reply { invite, accept: true }));
    b.until("friend A party status", |_, s| s.party.id != 0 && s.friends.friend(a_id).is_some_and(|f| f.in_party))
        .await?;
    a.until("friend B party status", |_, s| s.party.id != 0 && s.friends.friend(b_id).is_some_and(|f| f.in_party))
        .await?;
    b.client.send(Command::Party(PartyCommand::Leave { team_id: b.state.party.id, character_id: b_id }));
    a.until("friend B left party", |_, s| s.party.id == 0 && s.friends.friend(b_id).is_some_and(|f| !f.in_party))
        .await?;
    b.until("friend A left party", |_, s| s.party.id == 0 && s.friends.friend(a_id).is_some_and(|f| !f.in_party))
        .await?;
    println!("friend status: party join and disband propagated to both friends");

    a.send(FriendCommand::Block { name: b_name.clone() });
    a.until("block B", |_, s| s.friends.is_blocked(&b_name)).await?;
    a.send(FriendCommand::Unblock { name: b_name.clone() });
    a.until("unblock B", |_, s| !s.friends.is_blocked(&b_name) && s.friends.pending.is_empty()).await?;
    a.send(FriendCommand::Block { name: b_name.clone() });
    a.until("block B for persistence", |_, s| s.friends.is_blocked(&b_name)).await?;
    b.logout().await?;
    a.until("friend B offline", |_, s| s.friends.friend(b_id).is_some_and(|f| !f.online)).await?;
    println!("blocked: add/remove acknowledged; friend logout status received");

    a.logout().await?;
    a.reconnect().await?;
    ensure!(a.state.friends.friend(b_id).is_some_and(|f| !f.online && f.name == b_name));
    ensure!(a.state.friends.is_blocked(&b_name));
    ensure!(a.state.friends.total == 1 && a.state.friends.blocked_total == 1);
    b.reconnect().await?;
    a.until("friend B online again", |_, s| s.friends.friend(b_id).is_some_and(|f| f.online)).await?;
    ensure!(b.state.friends.friend(a_id).is_some_and(|f| f.online));
    println!("reconnect: saved friend/offline and blocked lists survived spawn; online transition received");

    a.send(FriendCommand::Unblock { name: b_name.clone() });
    a.until("final unblock", |_, s| s.friends.blocked.is_empty() && s.friends.pending.is_empty()).await?;
    a.send(FriendCommand::Remove { name: b_name });
    a.until("remove friend A", |_, s| s.friends.friends.is_empty() && s.friends.pending.is_empty()).await?;
    b.send(FriendCommand::Remove { name: a_name });
    b.until("remove friend B", |_, s| s.friends.friends.is_empty() && s.friends.pending.is_empty()).await?;
    a.logout().await?;
    a.reconnect().await?;
    ensure!(a.state.friends.friends.is_empty() && a.state.friends.blocked.is_empty());
    ensure!(a.state.friends.total == 0 && a.state.friends.blocked_total == 0);
    println!("remove persistence: empty login lists confirmed without list packets");
    Ok(())
}

#[tokio::test]
#[ignore = "requires bevy lane; creates two throw-away accounts and reconnects only its own sessions"]
async fn friends_blocking_presence_and_saved_lists() -> Result<()> {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let mut a = Peer::fresh(stamp, "a").await?;
    let mut b = match Peer::fresh(stamp, "b").await {
        Ok(peer) => peer,
        Err(error) => {
            let _ = a.logout().await;
            return Err(error);
        }
    };
    let result = exercise(&mut a, &mut b).await;
    let (out_a, out_b) = tokio::join!(a.logout(), b.logout());
    result?;
    out_a?;
    out_b?;
    println!("both social test clients logged out cleanly");
    Ok(())
}
